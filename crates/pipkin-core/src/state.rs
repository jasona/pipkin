use crate::ids::*;
use crate::model::*;
use crate::protocol::*;

/// Largest tool output kept for display, in bytes.
pub const TOOL_OUTPUT_PREVIEW_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug)]
pub struct ConversationState {
    pub id: ConversationId,
    pub project: ProjectId,
    pub title: String,
    pub updated_at: i64,
    pub items: Vec<TranscriptItem>,
    pub has_older: bool,
    pub loading_older: bool,
    pub opened: bool,
    /// Attachment generation. Events carrying another generation are dropped.
    pub generation: u64,
    pub run: RunState,
    pub queue: Vec<QueuedPrompt>,
    pub draft: Draft,
    pub changes: Vec<FileChange>,
    pub selected_change: Option<usize>,
    /// Text of the last submission, kept for explicit retry.
    pub last_submission: Option<(String, Vec<Attachment>)>,
    streaming_item: Option<usize>,
}

impl ConversationState {
    pub fn new(id: ConversationId, project: ProjectId, title: String, updated_at: i64) -> Self {
        ConversationState {
            id,
            project,
            title,
            updated_at,
            items: Vec::new(),
            has_older: false,
            loading_older: false,
            opened: false,
            generation: 0,
            run: RunState::Idle,
            queue: Vec::new(),
            draft: Draft::default(),
            changes: Vec::new(),
            selected_change: None,
            last_submission: None,
            streaming_item: None,
        }
    }

    /// Short activity label for the navigation list.
    pub fn activity(&self) -> Option<&'static str> {
        match self.run {
            RunState::Idle => None,
            ref r => Some(r.label()),
        }
    }
}

pub struct AppState {
    pub projects: Vec<Project>,
    pub models: Vec<ModelInfo>,
    pub conversations: Vec<ConversationState>,
    pub selected: Option<ConversationId>,
    pub search: String,
    pub prefs: Prefs,
    next_op: u64,
    next_local_item: u64,
    next_queue: u64,
    next_conversation: u64,
    now: i64,
}

pub struct Bootstrap {
    pub projects: Vec<Project>,
    pub models: Vec<ModelInfo>,
    pub conversations: Vec<(ConversationId, ProjectId, String, i64)>,
    pub now: i64,
}

impl AppState {
    pub fn new(boot: Bootstrap, prefs: Prefs) -> Self {
        let next_conversation = boot.conversations.iter().map(|c| c.0.0).max().unwrap_or(0) + 1;
        let conversations: Vec<_> = boot
            .conversations
            .into_iter()
            .map(|(id, p, t, at)| ConversationState::new(id, p, t, at))
            .collect();
        let mut prefs = prefs;
        if prefs.model.is_none() {
            prefs.model = boot.models.first().map(|m| m.id.clone());
        }
        let selected = prefs
            .selected_conversation
            .filter(|id| conversations.iter().any(|c| c.id == *id))
            .or_else(|| conversations.first().map(|c| c.id));
        AppState {
            projects: boot.projects,
            models: boot.models,
            conversations,
            selected,
            search: String::new(),
            prefs,
            next_op: 1,
            next_local_item: LOCAL_ITEM_BASE,
            next_queue: 1,
            next_conversation,
            now: boot.now,
        }
    }

    pub fn now(&self) -> i64 {
        self.now
    }

    pub fn set_now(&mut self, now: i64) {
        self.now = now;
    }

    pub fn conversation(&self, id: ConversationId) -> Option<&ConversationState> {
        self.conversations.iter().find(|c| c.id == id)
    }

    fn conv_mut(&mut self, id: ConversationId) -> Option<&mut ConversationState> {
        self.conversations.iter_mut().find(|c| c.id == id)
    }

    pub fn current(&self) -> Option<&ConversationState> {
        self.selected.and_then(|id| self.conversation(id))
    }

    pub fn current_project(&self) -> Option<&Project> {
        let p = self
            .prefs
            .selected_project
            .or(self.current()?.project.into())?;
        self.projects.iter().find(|x| x.id == p)
    }

    /// Conversations of the selected project that match the search, newest first.
    pub fn visible_conversations(&self) -> Vec<&ConversationState> {
        let project = self.current_project().map(|p| p.id);
        let q = self.search.trim().to_lowercase();
        let mut v: Vec<_> = self
            .conversations
            .iter()
            .filter(|c| Some(c.project) == project)
            .filter(|c| q.is_empty() || c.title.to_lowercase().contains(&q))
            .collect();
        v.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        v
    }

    pub fn availability(&self) -> Availability {
        let Some(c) = self.current() else {
            return Availability::default();
        };
        let has_text = !c.draft.text.trim().is_empty();
        let attachments_ok = c.draft.attachments.iter().all(|a| a.error.is_none());
        let ready = has_text && attachments_ok;
        let idle = matches!(c.run, RunState::Idle | RunState::Failed { .. });
        Availability {
            submit: idle && ready,
            steer: matches!(c.run, RunState::Running { .. }) && has_text,
            queue: matches!(
                c.run,
                RunState::Running { .. } | RunState::Submitting { .. }
            ) && ready,
            cancel: matches!(
                c.run,
                RunState::Running { .. } | RunState::Submitting { .. }
            ),
            check_status: matches!(c.run, RunState::OutcomeUnknown { .. }),
            retry: c.last_submission.is_some()
                && (matches!(c.run, RunState::Failed { .. })
                    || matches!(
                        c.items.last().map(|i| &i.kind),
                        Some(ItemKind::User {
                            delivery: Delivery::Rejected,
                            ..
                        })
                    )),
            load_older: c.has_older && !c.loading_older,
        }
    }

    fn alloc_op(&mut self) -> OperationId {
        let id = OperationId(self.next_op);
        self.next_op += 1;
        id
    }

    fn alloc_item(&mut self) -> ItemId {
        let id = ItemId(self.next_local_item);
        self.next_local_item += 1;
        id
    }

    fn push_item(
        &mut self,
        conv: ConversationId,
        kind: ItemKind,
        out: &mut Outcome,
    ) -> Option<usize> {
        let id = self.alloc_item();
        let at = self.now;
        let c = self.conv_mut(conv)?;
        c.items.push(TranscriptItem { id, at, kind });
        out.notes.push(Note::ItemsAppended(conv, 1));
        Some(c.items.len() - 1)
    }

    // ---------------------------------------------------------------- commands

    pub fn dispatch(&mut self, cmd: Command) -> Outcome {
        let mut out = Outcome::default();
        match cmd {
            Command::SelectProject(p) => {
                self.prefs.selected_project = Some(p);
                let first = self.visible_conversations().first().map(|c| c.id);
                match first {
                    Some(id) => out.merge(self.dispatch(Command::SelectConversation(id))),
                    None => {
                        self.selected = None;
                        out.notes.push(Note::SelectionChanged);
                    }
                }
                out.effects.push(Effect::SavePrefs(self.prefs.clone()));
            }
            Command::SelectConversation(id) => {
                let Some(c) = self.conv_mut(id) else {
                    return out;
                };
                let project = c.project;
                let prev = self.selected;
                // Flush the outgoing draft before leaving it.
                if let Some(prev) = prev.filter(|p| *p != id) {
                    out.merge(self.flush_draft(prev));
                }
                self.selected = Some(id);
                self.prefs.selected_project = Some(project);
                self.prefs.selected_conversation = Some(id);
                let c = self.conv_mut(id).unwrap();
                if !c.opened {
                    c.generation += 1;
                    out.effects.push(Effect::Backend(BackendRequest::Open {
                        conversation: id,
                        generation: c.generation,
                    }));
                }
                out.notes.push(Note::SelectionChanged);
                out.effects.push(Effect::SavePrefs(self.prefs.clone()));
            }
            Command::NewConversation => {
                let Some(project) = self.current_project().map(|p| p.id) else {
                    return out;
                };
                let id = ConversationId(self.next_conversation);
                self.next_conversation += 1;
                let mut c =
                    ConversationState::new(id, project, "New conversation".into(), self.now);
                c.opened = true;
                self.conversations.push(c);
                out.effects
                    .push(Effect::SaveConversation { conversation: id });
                out.notes.push(Note::ConversationsChanged);
                out.merge(self.dispatch(Command::SelectConversation(id)));
            }
            Command::RenameConversation(id, title) => {
                let title = title.trim().to_string();
                if title.is_empty() {
                    return out;
                }
                if let Some(c) = self.conv_mut(id) {
                    c.title = title;
                    out.effects
                        .push(Effect::SaveConversation { conversation: id });
                    out.notes.push(Note::ConversationsChanged);
                }
            }
            Command::SetSearch(q) => {
                self.search = q;
                out.notes.push(Note::ConversationsChanged);
            }

            Command::EditDraft(text) => {
                if let Some(id) = self.selected {
                    let c = self.conv_mut(id).unwrap();
                    if c.draft.text != text {
                        c.draft.text = text;
                        c.draft.rev += 1;
                        c.draft.save = SaveState::Dirty;
                        out.notes.push(Note::Other);
                    }
                }
            }
            Command::FlushDraft(id) => out.merge(self.flush_draft(id)),
            Command::AddAttachments(list) => {
                if let Some(id) = self.selected {
                    let c = self.conv_mut(id).unwrap();
                    for a in list {
                        if !c.draft.attachments.iter().any(|x| x.path == a.path) {
                            c.draft.attachments.push(a);
                        }
                    }
                    c.draft.rev += 1;
                    c.draft.save = SaveState::Dirty;
                    out.notes.push(Note::Other);
                }
            }
            Command::RemoveAttachment(i) => {
                if let Some(id) = self.selected {
                    let c = self.conv_mut(id).unwrap();
                    if i < c.draft.attachments.len() {
                        c.draft.attachments.remove(i);
                        out.notes.push(Note::Other);
                    }
                }
            }
            Command::SetModel(m) => {
                if self.models.iter().any(|x| x.id == m) {
                    self.prefs.model = Some(m);
                    out.effects.push(Effect::SavePrefs(self.prefs.clone()));
                    out.notes.push(Note::Other);
                }
            }

            Command::Submit => {
                if self.availability().submit {
                    out.merge(self.submit_from_draft());
                }
            }
            Command::Steer => {
                if self.availability().steer {
                    out.merge(self.steer());
                }
            }
            Command::QueueFollowUp => {
                if self.availability().queue {
                    let id = self.selected.unwrap();
                    let qid = QueueId(self.next_queue);
                    self.next_queue += 1;
                    let c = self.conv_mut(id).unwrap();
                    let text = std::mem::take(&mut c.draft.text);
                    c.draft.attachments.clear();
                    c.queue.push(QueuedPrompt { id: qid, text });
                    out.merge(self.reset_draft(id));
                }
            }
            Command::RemoveQueued(q) => {
                if let Some(id) = self.selected {
                    let c = self.conv_mut(id).unwrap();
                    c.queue.retain(|x| x.id != q);
                    out.notes.push(Note::Other);
                }
            }
            Command::Cancel => {
                if self.availability().cancel {
                    let id = self.selected.unwrap();
                    let c = self.conv_mut(id).unwrap();
                    let op = c.run.op().unwrap();
                    c.run = RunState::Stopping { op };
                    let generation = c.generation;
                    out.effects.push(Effect::Backend(BackendRequest::Cancel {
                        conversation: id,
                        generation,
                        op,
                    }));
                    out.notes.push(Note::Other);
                }
            }
            Command::CheckStatus => {
                if self.availability().check_status {
                    let id = self.selected.unwrap();
                    let c = self.conv(id);
                    out.effects
                        .push(Effect::Backend(BackendRequest::CheckStatus {
                            conversation: id,
                            generation: c.generation,
                            op: c.run.op().unwrap(),
                        }));
                }
            }
            Command::Retry => {
                if self.availability().retry {
                    let id = self.selected.unwrap();
                    let c = self.conv_mut(id).unwrap();
                    if let Some((text, atts)) = c.last_submission.clone() {
                        // Drop the rejected prompt row before resubmitting.
                        if let Some(ItemKind::User {
                            delivery: Delivery::Rejected,
                            ..
                        }) = c.items.last().map(|i| &i.kind)
                        {
                            c.items.pop();
                            out.notes.push(Note::ItemsReset(id));
                        }
                        c.run = RunState::Idle;
                        out.merge(self.submit(id, text, atts));
                    }
                }
            }
            Command::DismissFailure => {
                if let Some(id) = self.selected {
                    let c = self.conv_mut(id).unwrap();
                    if matches!(c.run, RunState::Failed { .. }) {
                        c.run = RunState::Idle;
                        out.notes.push(Note::Other);
                    }
                }
            }

            Command::LoadOlder => {
                if self.availability().load_older {
                    let id = self.selected.unwrap();
                    let c = self.conv_mut(id).unwrap();
                    c.loading_older = true;
                    let before = c.items.first().map(|i| i.id);
                    let generation = c.generation;
                    out.effects.push(Effect::Backend(BackendRequest::LoadOlder {
                        conversation: id,
                        generation,
                        before,
                    }));
                }
            }
            Command::SelectChange(i) => {
                if let Some(id) = self.selected {
                    let c = self.conv_mut(id).unwrap();
                    if i < c.changes.len() {
                        c.selected_change = Some(i);
                        self.prefs.inspector_open = true;
                        out.notes.push(Note::Other);
                    }
                }
            }

            Command::SetTheme(t) => self.pref(&mut out, |p| p.theme = t),
            Command::SetTextSize(t) => self.pref(&mut out, |p| p.text_size = t),
            Command::SetReducedMotion(v) => self.pref(&mut out, |p| p.reduced_motion = v),
            Command::SetNavWidth(w) => self.pref(&mut out, |p| p.nav_width = w.clamp(180.0, 420.0)),
            Command::SetInspectorWidth(w) => {
                self.pref(&mut out, |p| p.inspector_width = w.clamp(280.0, 720.0))
            }
            Command::SetInspectorOpen(v) => self.pref(&mut out, |p| p.inspector_open = v),
        }
        out
    }

    fn pref(&mut self, out: &mut Outcome, f: impl FnOnce(&mut Prefs)) {
        f(&mut self.prefs);
        out.effects.push(Effect::SavePrefs(self.prefs.clone()));
        out.notes.push(Note::Other);
    }

    fn conv(&self, id: ConversationId) -> &ConversationState {
        self.conversation(id).expect("conversation exists")
    }

    fn flush_draft(&mut self, id: ConversationId) -> Outcome {
        let mut out = Outcome::default();
        if let Some(c) = self.conv_mut(id)
            && c.draft.save == SaveState::Dirty
        {
            c.draft.save = SaveState::Saving;
            out.effects.push(Effect::SaveDraft {
                conversation: id,
                text: c.draft.text.clone(),
                rev: c.draft.rev,
            });
            out.notes.push(Note::Other);
        }
        out
    }

    /// Core-originated draft change: the editor must resync.
    fn reset_draft(&mut self, id: ConversationId) -> Outcome {
        let c = self.conv_mut(id).unwrap();
        c.draft.rev += 1;
        c.draft.sync_epoch += 1;
        c.draft.save = SaveState::Dirty;
        let mut out = self.flush_draft(id);
        out.notes.push(Note::Other);
        out
    }

    fn submit_from_draft(&mut self) -> Outcome {
        let id = self.selected.unwrap();
        let c = self.conv_mut(id).unwrap();
        let text = c.draft.text.trim_end().to_string();
        let atts = std::mem::take(&mut c.draft.attachments);
        c.draft.text.clear();
        let mut out = self.reset_draft(id);
        out.merge(self.submit(id, text, atts));
        out
    }

    fn submit(&mut self, id: ConversationId, text: String, atts: Vec<Attachment>) -> Outcome {
        let mut out = Outcome::default();
        let op = self.alloc_op();
        let model = self.prefs.model.clone();
        let now = self.now;
        let c = self.conv_mut(id).unwrap();
        c.run = RunState::Submitting { op };
        c.last_submission = Some((text.clone(), atts.clone()));
        c.updated_at = now;
        c.streaming_item = None;
        let generation = c.generation;
        self.push_item(
            id,
            ItemKind::User {
                text: text.clone(),
                attachments: atts.clone(),
                delivery: Delivery::Pending,
                steer: false,
            },
            &mut out,
        );
        out.effects.push(Effect::Backend(BackendRequest::Submit {
            conversation: id,
            generation,
            op,
            text,
            attachments: atts,
            model,
        }));
        out.effects
            .push(Effect::SaveConversation { conversation: id });
        out.notes.push(Note::ConversationsChanged);
        out
    }

    fn steer(&mut self) -> Outcome {
        let mut out = Outcome::default();
        let id = self.selected.unwrap();
        let c = self.conv_mut(id).unwrap();
        let text = std::mem::take(&mut c.draft.text).trim_end().to_string();
        let op = c.run.op().unwrap();
        let generation = c.generation;
        c.streaming_item = None;
        self.push_item(
            id,
            ItemKind::User {
                text: text.clone(),
                attachments: vec![],
                delivery: Delivery::Pending,
                steer: true,
            },
            &mut out,
        );
        out.effects.push(Effect::Backend(BackendRequest::Steer {
            conversation: id,
            generation,
            op,
            text,
        }));
        out.merge(self.reset_draft(id));
        out
    }

    // ------------------------------------------------------------------ events

    pub fn apply_event(&mut self, ev: BackendEvent) -> Outcome {
        let mut out = Outcome::default();
        let id = ev.conversation;
        let Some(c) = self.conv_mut(id) else {
            return out;
        };
        // Stale generation: an earlier attachment's event must never land here.
        if ev.generation != c.generation {
            return out;
        }
        // Operation-scoped events must match the live operation.
        let scoped = !matches!(
            ev.kind,
            EventKind::Opened { .. } | EventKind::OlderPage { .. }
        );
        if scoped && ev.op != c.run.op() {
            return out;
        }
        match ev.kind {
            EventKind::Opened {
                items,
                has_older,
                changes,
            } => {
                c.items = items;
                c.has_older = has_older;
                c.opened = true;
                c.selected_change = (!changes.is_empty()).then_some(0);
                c.changes = changes;
                out.notes.push(Note::ItemsReset(id));
            }
            EventKind::OlderPage { items, has_older } => {
                let n = items.len();
                c.loading_older = false;
                c.has_older = has_older;
                let mut v = items;
                v.append(&mut c.items);
                c.items = v;
                if let Some(s) = c.streaming_item.as_mut() {
                    *s += n;
                }
                out.notes.push(Note::ItemsPrepended(id, n));
            }
            EventKind::Accepted => {
                if let RunState::Submitting { op } = c.run {
                    c.run = RunState::Running { op };
                    mark_last_user(c, Delivery::Sent, &mut out, id);
                }
            }
            EventKind::SteerAccepted => {
                mark_last_user(c, Delivery::Sent, &mut out, id);
            }
            EventKind::Rejected { reason } => {
                if let RunState::Submitting { .. } = c.run {
                    mark_last_user(c, Delivery::Rejected, &mut out, id);
                    c.run = RunState::Failed {
                        message: reason.clone(),
                    };
                    // Retain the user's text: restore it to the draft if the editor is empty.
                    if c.draft.text.is_empty()
                        && let Some((t, a)) = c.last_submission.clone()
                    {
                        c.draft.text = t;
                        c.draft.attachments = a;
                        c.draft.sync_epoch += 1;
                        c.draft.rev += 1;
                        c.draft.save = SaveState::Dirty;
                    }
                    self.push_item(
                        id,
                        ItemKind::Notice {
                            text: reason,
                            level: NoticeLevel::Error,
                        },
                        &mut out,
                    );
                }
            }
            EventKind::AckLost => {
                if let RunState::Submitting { op } = c.run {
                    c.run = RunState::OutcomeUnknown { op };
                    mark_last_user(c, Delivery::Unknown, &mut out, id);
                    out.notes.push(Note::Other);
                }
            }
            EventKind::StatusResolved { accepted } => {
                if let RunState::OutcomeUnknown { op } = c.run {
                    if accepted {
                        c.run = RunState::Running { op };
                        mark_last_user(c, Delivery::Sent, &mut out, id);
                    } else {
                        mark_last_user(c, Delivery::Rejected, &mut out, id);
                        c.run = RunState::Failed {
                            message: "The engine never received the prompt.".into(),
                        };
                    }
                    out.notes.push(Note::Other);
                }
            }
            EventKind::Token(t) => match c.streaming_item {
                Some(i) => {
                    if let ItemKind::Assistant { text, .. } = &mut c.items[i].kind {
                        text.push_str(&t);
                    }
                    out.notes.push(Note::ItemChanged(id, i));
                }
                None => {
                    let idx = self.push_item(
                        id,
                        ItemKind::Assistant {
                            text: t,
                            streaming: true,
                        },
                        &mut out,
                    );
                    self.conv_mut(id).unwrap().streaming_item = idx;
                }
            },
            EventKind::ToolStarted { call, name, input } => {
                self.finish_streaming(id, &mut out);
                let op = ev.op.unwrap();
                self.push_item(
                    id,
                    ItemKind::Tool(ToolCall {
                        call_ref: Some((op, call)),
                        name,
                        input,
                        output: String::new(),
                        truncated: false,
                        full_len: 0,
                        status: ToolStatus::Running,
                    }),
                    &mut out,
                );
            }
            EventKind::ToolOutput { call, chunk } => {
                let op = ev.op.unwrap();
                if let Some((i, t)) = find_tool(c, op, call) {
                    t.full_len += chunk.len();
                    if t.output.len() < TOOL_OUTPUT_PREVIEW_BYTES {
                        let room = TOOL_OUTPUT_PREVIEW_BYTES - t.output.len();
                        let cut = floor_boundary(&chunk, room);
                        t.output.push_str(&chunk[..cut]);
                        if cut < chunk.len() {
                            t.truncated = true;
                        }
                    } else {
                        t.truncated = true;
                    }
                    out.notes.push(Note::ItemChanged(id, i));
                }
            }
            EventKind::ToolFinished { call, ok } => {
                let op = ev.op.unwrap();
                if let Some((i, t)) = find_tool(c, op, call) {
                    t.status = if ok {
                        ToolStatus::Ok
                    } else {
                        ToolStatus::Failed
                    };
                    out.notes.push(Note::ItemChanged(id, i));
                }
            }
            EventKind::ChangesReported(changes) => {
                c.selected_change = c
                    .selected_change
                    .filter(|i| *i < changes.len())
                    .or((!changes.is_empty()).then_some(0));
                c.changes = changes;
                out.notes.push(Note::Other);
            }
            EventKind::Completed => {
                c.run = RunState::Idle;
                self.finish_streaming(id, &mut out);
                out.merge(self.after_settled(id));
            }
            EventKind::Failed { message } => {
                c.run = RunState::Failed {
                    message: message.clone(),
                };
                self.finish_streaming(id, &mut out);
                self.push_item(
                    id,
                    ItemKind::Notice {
                        text: message,
                        level: NoticeLevel::Error,
                    },
                    &mut out,
                );
                out.notes.push(Note::Other);
            }
            EventKind::Cancelled => {
                if matches!(
                    c.run,
                    RunState::Stopping { .. }
                        | RunState::Running { .. }
                        | RunState::Submitting { .. }
                ) {
                    c.run = RunState::Idle;
                    self.finish_streaming(id, &mut out);
                    self.push_item(
                        id,
                        ItemKind::Notice {
                            text: "Run stopped.".into(),
                            level: NoticeLevel::Info,
                        },
                        &mut out,
                    );
                    // Queued follow-ups stay queued after a user cancel.
                    out.notes.push(Note::Other);
                }
            }
        }
        out
    }

    fn finish_streaming(&mut self, id: ConversationId, out: &mut Outcome) {
        let c = self.conv_mut(id).unwrap();
        if let Some(i) = c.streaming_item.take() {
            if let ItemKind::Assistant { streaming, .. } = &mut c.items[i].kind {
                *streaming = false;
            }
            out.notes.push(Note::ItemChanged(id, i));
        }
    }

    /// After a run completes normally, start the next queued prompt.
    fn after_settled(&mut self, id: ConversationId) -> Outcome {
        let c = self.conv_mut(id).unwrap();
        if c.queue.is_empty() {
            return Outcome::default();
        }
        let next = c.queue.remove(0);
        self.submit(id, next.text, vec![])
    }

    // ------------------------------------------------------------- persistence

    pub fn draft_saved(&mut self, id: ConversationId, rev: u64, result: Result<(), String>) {
        if let Some(c) = self.conv_mut(id) {
            match result {
                Ok(()) if c.draft.rev == rev => c.draft.save = SaveState::Saved,
                Ok(()) => c.draft.save = SaveState::Dirty,
                Err(e) => c.draft.save = SaveState::Failed(e),
            }
        }
    }

    /// Restore a draft read from storage at startup.
    pub fn restore_draft(&mut self, id: ConversationId, text: String) {
        if let Some(c) = self.conv_mut(id)
            && !text.is_empty()
            && c.draft.text.is_empty()
        {
            c.draft.text = text;
            c.draft.sync_epoch += 1;
            c.draft.save = SaveState::Saved;
        }
    }

    pub fn restore_conversation(
        &mut self,
        id: ConversationId,
        project: ProjectId,
        title: String,
        at: i64,
    ) {
        if let Some(c) = self.conv_mut(id) {
            c.title = title;
            c.updated_at = at;
        } else {
            let mut c = ConversationState::new(id, project, title, at);
            c.opened = true;
            self.conversations.push(c);
            self.next_conversation = self.next_conversation.max(id.0 + 1);
        }
    }
}

fn mark_last_user(c: &mut ConversationState, d: Delivery, out: &mut Outcome, id: ConversationId) {
    if let Some((i, item)) = c
        .items
        .iter_mut()
        .enumerate()
        .rev()
        .find(|(_, it)| matches!(it.kind, ItemKind::User { .. }))
    {
        if let ItemKind::User { delivery, .. } = &mut item.kind {
            *delivery = d;
        }
        out.notes.push(Note::ItemChanged(id, i));
    }
}

fn find_tool(
    c: &mut ConversationState,
    op: OperationId,
    call: u32,
) -> Option<(usize, &mut ToolCall)> {
    c.items
        .iter_mut()
        .enumerate()
        .rev()
        .find_map(|(i, it)| match &mut it.kind {
            ItemKind::Tool(t) if t.call_ref == Some((op, call)) => Some((i, t)),
            _ => None,
        })
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
