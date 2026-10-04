use crate::ids::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub path: String,
    pub name: String,
    pub size: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    /// Submitted, waiting for acceptance.
    Pending,
    /// Acknowledgment lost; the engine may or may not have the prompt.
    Unknown,
    Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Ok,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub call_ref: Option<(OperationId, u32)>,
    pub name: String,
    pub input: String,
    pub output: String,
    /// Output was cut to the preview bound; `full_len` is the original size in bytes.
    pub truncated: bool,
    pub full_len: usize,
    pub status: ToolStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    User {
        text: String,
        attachments: Vec<Attachment>,
        delivery: Delivery,
        steer: bool,
    },
    Assistant {
        text: String,
        streaming: bool,
    },
    Tool(ToolCall),
    Notice {
        text: String,
        level: NoticeLevel,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptItem {
    pub id: ItemId,
    /// Unix seconds.
    pub at: i64,
    pub kind: ItemKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Context,
    Add,
    Remove,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<Hunk>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueuedPrompt {
    pub id: QueueId,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunState {
    Idle,
    /// Submitted; waiting for the engine to accept.
    Submitting {
        op: OperationId,
    },
    Running {
        op: OperationId,
    },
    /// Cancel requested; stays here until the engine confirms settlement.
    Stopping {
        op: OperationId,
    },
    /// Acknowledgment was lost. Never resent automatically.
    OutcomeUnknown {
        op: OperationId,
    },
    Failed {
        message: String,
    },
}

impl RunState {
    pub fn op(&self) -> Option<OperationId> {
        match self {
            RunState::Submitting { op }
            | RunState::Running { op }
            | RunState::Stopping { op }
            | RunState::OutcomeUnknown { op } => Some(*op),
            _ => None,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.op().is_some()
    }

    pub fn label(&self) -> &'static str {
        match self {
            RunState::Idle => "Idle",
            RunState::Submitting { .. } => "Sending",
            RunState::Running { .. } => "Working",
            RunState::Stopping { .. } => "Stopping",
            RunState::OutcomeUnknown { .. } => "Outcome unknown",
            RunState::Failed { .. } => "Failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SaveState {
    Clean,
    Dirty,
    Saving,
    Saved,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub rev: u64,
    /// Bumped whenever the core (not the editor) changes the text, so the editor resyncs.
    pub sync_epoch: u64,
    pub save: SaveState,
}

impl Default for Draft {
    fn default() -> Self {
        Draft {
            text: String::new(),
            attachments: Vec::new(),
            rev: 0,
            sync_epoch: 0,
            save: SaveState::Clean,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSize {
    Small,
    Normal,
    Large,
}

impl TextSize {
    pub fn scale(self) -> f32 {
        match self {
            TextSize::Small => 0.92,
            TextSize::Normal => 1.0,
            TextSize::Large => 1.2,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub theme: Theme,
    pub text_size: TextSize,
    pub reduced_motion: bool,
    pub nav_width: f32,
    pub inspector_width: f32,
    pub inspector_open: bool,
    pub selected_project: Option<ProjectId>,
    pub selected_conversation: Option<ConversationId>,
    pub model: Option<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: Theme::Dark,
            text_size: TextSize::Normal,
            reduced_motion: false,
            nav_width: 240.0,
            inspector_width: 400.0,
            inspector_open: true,
            selected_project: None,
            selected_conversation: None,
            model: None,
        }
    }
}
