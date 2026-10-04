use crate::ids::*;
use crate::model::*;

/// User intents issued by the UI or command palette.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    SelectProject(ProjectId),
    SelectConversation(ConversationId),
    NewConversation,
    RenameConversation(ConversationId, String),
    SetSearch(String),

    EditDraft(String),
    FlushDraft(ConversationId),
    AddAttachments(Vec<Attachment>),
    RemoveAttachment(usize),
    SetModel(String),

    Submit,
    Steer,
    QueueFollowUp,
    RemoveQueued(QueueId),
    Cancel,
    /// Ask the backend what happened to an acknowledgment-less submission.
    CheckStatus,
    /// Resubmit after a rejection or failure. Explicit user action only.
    Retry,
    DismissFailure,

    LoadOlder,
    SelectChange(usize),

    SetTheme(Theme),
    SetTextSize(TextSize),
    SetReducedMotion(bool),
    SetNavWidth(f32),
    SetInspectorWidth(f32),
    SetInspectorOpen(bool),
}

/// What the application can do right now. UI controls and the command palette share this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Availability {
    pub new_conversation: bool,
    pub submit: bool,
    pub steer: bool,
    pub queue: bool,
    pub cancel: bool,
    pub check_status: bool,
    pub retry: bool,
    pub load_older: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BackendRequest {
    Open {
        conversation: ConversationId,
        generation: u64,
    },
    LoadOlder {
        conversation: ConversationId,
        generation: u64,
        before: Option<ItemId>,
    },
    Submit {
        conversation: ConversationId,
        generation: u64,
        op: OperationId,
        /// Durable idempotency key, already journaled before this request is sent.
        request: RequestId,
        text: String,
        attachments: Vec<Attachment>,
        model: Option<String>,
    },
    Steer {
        conversation: ConversationId,
        generation: u64,
        op: OperationId,
        text: String,
    },
    Cancel {
        conversation: ConversationId,
        generation: u64,
        op: OperationId,
    },
    CheckStatus {
        conversation: ConversationId,
        generation: u64,
        op: OperationId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackendEvent {
    pub conversation: ConversationId,
    pub generation: u64,
    pub op: Option<OperationId>,
    pub kind: EventKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    /// Initial window of history for a conversation.
    Opened {
        items: Vec<TranscriptItem>,
        has_older: bool,
        changes: Vec<FileChange>,
    },
    OlderPage {
        items: Vec<TranscriptItem>,
        has_older: bool,
    },
    /// The backend's current view of an open conversation, replacing what is shown. Used for
    /// replicated state that changes after the initial window (live output, other writers).
    /// Ignored until the conversation has been opened.
    Synced {
        items: Vec<TranscriptItem>,
    },
    /// Opening the conversation failed. It stays unopened, so selecting it again retries.
    OpenFailed {
        message: String,
    },
    Accepted,
    /// The submission was refused before any work began.
    Rejected {
        reason: String,
    },
    /// The connection dropped before acknowledgment.
    AckLost,
    /// Answer to `CheckStatus`.
    StatusResolved {
        accepted: bool,
    },
    Token(String),
    ToolStarted {
        call: u32,
        name: String,
        input: String,
    },
    ToolOutput {
        call: u32,
        chunk: String,
    },
    ToolFinished {
        call: u32,
        ok: bool,
    },
    ChangesReported(Vec<FileChange>),
    Completed,
    Failed {
        message: String,
    },
    /// The engine confirms a cancel has settled.
    Cancelled,
    SteerAccepted,
}

/// Work the controller executes on the core's behalf.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Backend(BackendRequest),
    SaveDraft {
        conversation: ConversationId,
        text: String,
        rev: u64,
    },
    SavePrefs(Prefs),
    SaveConversation {
        conversation: ConversationId,
    },
    /// Durably record a submission's immutable payload. The controller must report the commit
    /// with `AppState::intent_persisted`; until then nothing is sent and the draft is kept.
    JournalIntent {
        conversation: ConversationId,
        request: RequestId,
        text: String,
        attachments: Vec<Attachment>,
        model: Option<String>,
    },
    JournalState {
        conversation: ConversationId,
        request: RequestId,
        state: JournalState,
    },
}

/// Lifecycle of a journaled request after its intent is recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalState {
    /// The acknowledgment was lost; whether the engine accepted it is unknown.
    Unknown,
    Accepted,
    Rejected,
    Completed,
    Failed,
    Cancelled,
    /// Replaced by a newer request for the same conversation during recovery.
    Superseded,
}

impl JournalState {
    /// No further reconciliation is needed once a request reaches a terminal state.
    pub fn is_terminal(self) -> bool {
        !matches!(self, JournalState::Unknown | JournalState::Accepted)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            JournalState::Unknown => "unknown",
            JournalState::Accepted => "accepted",
            JournalState::Rejected => "rejected",
            JournalState::Completed => "completed",
            JournalState::Failed => "failed",
            JournalState::Cancelled => "cancelled",
            JournalState::Superseded => "superseded",
        }
    }
}

/// Fine-grained change notices so views can splice instead of rebuilding.
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    ItemsReset(ConversationId),
    ItemsPrepended(ConversationId, usize),
    ItemsAppended(ConversationId, usize),
    ItemChanged(ConversationId, usize),
    ConversationsChanged,
    SelectionChanged,
    Other,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub effects: Vec<Effect>,
    pub notes: Vec<Note>,
}

impl Outcome {
    pub fn merge(&mut self, other: Outcome) {
        self.effects.extend(other.effects);
        self.notes.extend(other.notes);
    }
}
