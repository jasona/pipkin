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
    /// Steers and follow-ups sent to the engine and not yet confirmed (real mode).
    pub pending_queue: Vec<PendingQueued>,
    /// Known only from its saved copy: the engine has not (yet) listed it. It goes away when the
    /// engine's list arrives without it.
    pub cached_only: bool,
    /// What is shown is a saved copy from this time (Unix seconds), not the engine's own state.
    pub cached_at: Option<i64>,
    /// Why the engine's own state could not be had while a saved copy is shown.
    pub stale_reason: Option<String>,
    /// Why the last attempt to load older history failed.
    pub older_error: Option<String>,
    /// Questions extensions are waiting on, in the order asked.
    pub ui_requests: Vec<UiRequest>,
    /// Answers sent and not yet taken by the engine.
    pub ui_answering: Vec<String>,
    pub ui_status: Vec<(String, String)>,
    pub ui_notices: Vec<UiNotice>,
    /// Why the engine refused the last answer.
    pub ui_error: Option<String>,
    /// A complete tool output has been asked for and is on its way.
    pub pending_output: Option<(String, OutputUse)>,
    pub draft: Draft,
    pub changes: Vec<FileChange>,
    pub selected_change: Option<usize>,
    /// Text of the last submission, kept for explicit retry.
    pub last_submission: Option<(String, Vec<Attachment>)>,
    /// A submission waiting for its journal commit. While set, nothing else can be submitted.
    pub pending_intent: Option<PendingIntent>,
    /// Why the last submission could not be recorded; its draft was kept.
    pub intent_error: Option<String>,
    /// Request behind the live (or unresolved) operation, for journal updates.
    current_request: Option<RequestId>,
    streaming_item: Option<usize>,
    /// The run shown as `Running` was not started by this window (the engine reported it busy),
    /// so only the engine's own state can end it.
    adopted: Option<OperationId>,
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
            pending_queue: Vec::new(),
            cached_only: false,
            cached_at: None,
            stale_reason: None,
            older_error: None,
            ui_requests: Vec::new(),
            ui_answering: Vec::new(),
            ui_status: Vec::new(),
            ui_notices: Vec::new(),
            ui_error: None,
            pending_output: None,
            draft: Draft::default(),
            changes: Vec::new(),
            selected_change: None,
            last_submission: None,
            pending_intent: None,
            intent_error: None,
            current_request: None,
            streaming_item: None,
            adopted: None,
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
    pub mode: Mode,
    pub connection: Connection,
    /// Why this build cannot send prompts, if it cannot. Shown wherever sending is offered.
    pub read_only: Option<String>,
    /// Whether the backend can create conversations (real mode); demo mode creates them locally.
    pub can_create: bool,
    /// A background action failed and there is no conversation to show it on.
    pub notice: Option<String>,
    /// Saved state could not be read or written as usual; shown until dismissed.
    pub storage_issue: Option<String>,
    /// The result of searching saved history for the text in the search box.
    pub history_search: SearchResults,
    /// The conversation to go back to after opening a search hit.
    pub search_return: Option<ConversationId>,
    /// A message the view is to scroll to once its conversation shows it.
    pub scroll_target: Option<(ConversationId, ItemId)>,
    target_pages: u32,
    /// Project folders the user opened, kept across catalogs.
    bookmarks: Vec<Project>,
    /// A conversation creation waiting for the backend to report it.
    pending_create: Option<RequestId>,
    request_prefix: String,
    next_request: u64,
    next_op: u64,
    next_local_item: u64,
    next_queue: u64,
    next_conversation: u64,
    now: i64,
}

#[derive(Clone, Debug, PartialEq)]
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
            mode: Mode::Demo,
            connection: Connection::Ready,
            read_only: None,
            can_create: false,
            notice: None,
            storage_issue: None,
            history_search: SearchResults::default(),
            search_return: None,
            scroll_target: None,
            target_pages: 0,
            bookmarks: Vec::new(),
            pending_create: None,
            request_prefix: "req".into(),
            next_request: 1,
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
        // Only consult the current conversation when no project is selected: `current()?`
        // must not run eagerly, or an empty project would report no project at all.
        let p = match self.prefs.selected_project {
            Some(p) => p,
            None => self.current()?.project,
        };
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

    /// Merge the backend's catalog into state. Existing conversations keep their local state.
    pub fn apply_catalog(&mut self, boot: Bootstrap) -> Outcome {
        let mut out = Outcome::default();
        self.projects = merge_projects(boot.projects, &self.bookmarks);
        self.models = boot.models;
        self.now = self.now.max(boot.now);
        // The engine's list is the truth: a conversation known only from a saved copy stays
        // only if the engine lists it.
        let listed: std::collections::HashSet<ConversationId> =
            boot.conversations.iter().map(|c| c.0).collect();
        self.conversations
            .retain(|c| !c.cached_only || listed.contains(&c.id));
        for c in &mut self.conversations {
            if listed.contains(&c.id) {
                c.cached_only = false;
            }
        }
        if self
            .selected
            .is_some_and(|id| self.conversation(id).is_none())
        {
            self.selected = None;
        }
        let mut created: Vec<(ConversationId, i64)> = Vec::new();
        for (id, project, title, at) in boot.conversations {
            if self.conv_mut(id).is_none() {
                self.conversations
                    .push(ConversationState::new(id, project, title, at));
                self.next_conversation = self.next_conversation.max(id.0 + 1);
                created.push((id, at));
            }
        }
        // A conversation we asked the backend to create has arrived: show it.
        if self.pending_create.is_some()
            && let Some((id, _)) = created.iter().max_by_key(|(_, at)| *at)
        {
            let id = *id;
            self.pending_create = None;
            out.merge(self.dispatch(Command::SelectConversation(id)));
        }
        if self.prefs.model.is_none() {
            self.prefs.model = self.models.first().map(|m| m.id.clone());
        }
        out.notes.push(Note::ConversationsChanged);
        out
    }

    /// The engine reports which model is selected; that, not a local choice, is what is shown.
    pub fn set_engine_model(&mut self, model: Option<String>) -> Outcome {
        let mut out = Outcome::default();
        if model.is_some() && self.prefs.model != model {
            self.prefs.model = model;
            out.notes.push(Note::Other);
        }
        out
    }

    /// Show a failure that has no conversation to attach to, and stop waiting on anything it ended.
    pub fn set_notice(&mut self, message: String) -> Outcome {
        self.pending_create = None;
        self.notice = Some(message);
        Outcome {
            notes: vec![Note::Other],
            ..Outcome::default()
        }
    }

    /// Restore a project folder remembered from an earlier run (no effects).
    pub fn restore_project(&mut self, path: &str) {
        let id = project_id_for_path(path);
        if self.bookmarks.iter().all(|p| p.id != id) {
            self.bookmarks.push(Project {
                id,
                name: project_name_for_path(path),
                path: path.to_owned(),
            });
        }
        self.projects = merge_projects(std::mem::take(&mut self.projects), &self.bookmarks);
    }

    /// Select the remembered (or first) conversation, if nothing is selected, so its history loads.
    pub fn select_initial(&mut self) -> Outcome {
        if self.selected.is_some() {
            return Outcome::default();
        }
        let id = self
            .prefs
            .selected_conversation
            .filter(|id| self.conversation(*id).is_some())
            .or_else(|| self.conversations.first().map(|c| c.id));
        match id {
            Some(id) => self.dispatch(Command::SelectConversation(id)),
            None => Outcome::default(),
        }
    }

    pub fn set_connection(&mut self, connection: Connection) -> Outcome {
        let mut out = Outcome::default();
        if self.connection != connection {
            let was_ready = self.connection.is_ready();
            self.connection = connection;
            // Questions from extensions cannot be answered without the engine; its own state is
            // read again when it is reachable.
            if !self.connection.is_ready() {
                for c in &mut self.conversations {
                    c.ui_requests.clear();
                    c.ui_answering.clear();
                }
            }
            out.notes.push(Note::Other);
            // Asking the engine what became of a submission is read-only, so it needs no
            // confirmation once the engine is reachable again.
            if !was_ready && self.connection.is_ready() {
                out.merge(self.check_unknown());
            }
        }
        out
    }

    /// Ask about the submission in doubt in the open conversation, if there is one.
    fn check_unknown(&mut self) -> Outcome {
        let mut out = Outcome::default();
        let Some(c) = self.current() else { return out };
        if c.opened
            && let RunState::OutcomeUnknown { op } = c.run
        {
            out.effects
                .push(Effect::Backend(BackendRequest::CheckStatus {
                    conversation: c.id,
                    generation: c.generation,
                    op,
                    request: c.current_request.clone(),
                }));
        }
        out
    }

    /// Record that saved state could not be used as usual.
    pub fn set_storage_issue(&mut self, message: String) -> Outcome {
        self.storage_issue = Some(message);
        Outcome {
            notes: vec![Note::Other],
            ..Outcome::default()
        }
    }

    pub fn availability(&self) -> Availability {
        // Every action below reaches the backend, so none is offered until it is connected.
        if !self.connection.is_ready() {
            return Availability::default();
        }
        // Real sessions are created by the engine, so creation is offered only once it can be asked.
        let new_conversation = (self.mode == Mode::Demo || self.can_create)
            && self.pending_create.is_none()
            && self.current_project().is_some();
        let Some(c) = self.current() else {
            return Availability {
                new_conversation,
                ..Availability::default()
            };
        };
        let has_text = !c.draft.text.trim().is_empty();
        let attachments_ok = c.draft.attachments.iter().all(|a| a.error.is_none());
        let ready = has_text && attachments_ok;
        let idle =
            matches!(c.run, RunState::Idle | RunState::Failed { .. }) && c.pending_intent.is_none();
        // A read-only build offers no way to send, steer, queue or retry.
        let can_send = self.read_only.is_none();
        // Real steers and follow-ups are journaled one at a time, like prompts.
        let intent_free = self.mode == Mode::Demo || c.pending_intent.is_none();
        Availability {
            new_conversation,
            // A conversation showing only a saved copy cannot be written to.
            submit: can_send && idle && ready && (self.mode == Mode::Demo || c.opened),
            steer: can_send && intent_free && matches!(c.run, RunState::Running { .. }) && ready,
            queue: can_send
                && intent_free
                && matches!(
                    c.run,
                    RunState::Running { .. } | RunState::Submitting { .. }
                )
                && ready,
            refresh_models: self.mode == Mode::Real && c.opened,
            retry_save: matches!(c.draft.save, SaveState::Failed(_)),
            open_in_editor: self.mode == Mode::Real
                && self.current_project().is_some()
                && c.selected_change.is_some_and(|i| i < c.changes.len()),
            open_terminal: self.mode == Mode::Real && self.current_project().is_some(),
            cancel: matches!(
                c.run,
                RunState::Running { .. } | RunState::Submitting { .. }
            ),
            check_status: matches!(c.run, RunState::OutcomeUnknown { .. }),
            retry: can_send
                && c.last_submission.is_some()
                && c.pending_intent.is_none()
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

    /// Prefix that makes request IDs unique across restarts (the controller supplies entropy;
    /// the core never reads a clock or random source).
    pub fn set_request_prefix(&mut self, prefix: String) {
        self.request_prefix = prefix;
    }

    fn alloc_request(&mut self) -> RequestId {
        let id = RequestId(format!("{}-{}", self.request_prefix, self.next_request));
        self.next_request += 1;
        id
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
                let real = self.mode == Mode::Real;
                let prev = self.selected;
                let Some(c) = self.conv_mut(id) else {
                    return out;
                };
                let project = c.project;
                // Flush the outgoing draft before leaving it.
                if let Some(prev) = prev.filter(|p| *p != id) {
                    out.merge(self.flush_draft(prev));
                }
                self.selected = Some(id);
                self.prefs.selected_project = Some(project);
                self.prefs.selected_conversation = Some(id);
                let c = self.conv_mut(id).unwrap();
                // A real engine is attached to one session at a time, so coming back to a
                // conversation attaches it again; the demo keeps what it already loaded.
                if !c.opened || (real && prev != Some(id)) {
                    c.generation += 1;
                    let generation = c.generation;
                    // Until the engine answers, a saved copy is better than an empty screen.
                    if real && !c.opened && c.items.is_empty() {
                        out.effects.push(Effect::LoadCache { conversation: id });
                    }
                    out.effects.push(Effect::Backend(BackendRequest::Open {
                        conversation: id,
                        generation,
                    }));
                }
                out.notes.push(Note::SelectionChanged);
                out.effects.push(Effect::SavePrefs(self.prefs.clone()));
            }
            // Real sessions are created and renamed by the engine; a local-only row would be
            // simulated success.
            Command::RenameConversation(..) if self.mode == Mode::Real => {}
            Command::NewConversation if self.mode == Mode::Real => {
                if self.availability().new_conversation
                    && let Some(project) = self.current_project().cloned()
                {
                    let request = self.alloc_request();
                    self.pending_create = Some(request.clone());
                    out.effects
                        .push(Effect::Backend(BackendRequest::CreateConversation {
                            project: project.id,
                            cwd: project.path,
                            request,
                        }));
                    out.notes.push(Note::Other);
                }
            }
            Command::AddProject(path) => {
                let path = path.trim().trim_end_matches('/').to_string();
                if !path.starts_with('/') {
                    return self.set_notice("A project folder must be an absolute path.".into());
                }
                let id = project_id_for_path(&path);
                if self.bookmarks.iter().all(|p| p.id != id) {
                    out.effects.push(Effect::SaveProject { path: path.clone() });
                }
                self.restore_project(&path);
                out.notes.push(Note::ConversationsChanged);
                out.merge(self.dispatch(Command::SelectProject(id)));
            }
            Command::DismissNotice => {
                if self.notice.take().is_some() {
                    out.notes.push(Note::Other);
                }
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
                // Titles are filtered at once; saved messages are searched by the controller.
                let query = self.search.trim().to_owned();
                if self.mode == Mode::Real && query.chars().count() >= 2 {
                    out.effects.push(Effect::SearchHistory { query });
                } else if !self.history_search.query.is_empty()
                    || !self.history_search.hits.is_empty()
                {
                    self.history_search = SearchResults::default();
                }
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
                        c.draft.rev += 1;
                        c.draft.save = SaveState::Dirty;
                        out.notes.push(Note::Other);
                    }
                }
            }
            Command::SetModel(m) => {
                if self.models.iter().any(|x| x.id == m) {
                    if self.mode == Mode::Real {
                        // Ask the engine; the selection changes only when it reports it.
                        if let Some(id) = self.selected.filter(|id| self.conv(*id).opened) {
                            out.effects.push(Effect::Backend(BackendRequest::SetModel {
                                conversation: id,
                                generation: self.conv(id).generation,
                                model: m,
                            }));
                        }
                    } else {
                        self.prefs.model = Some(m);
                        out.effects.push(Effect::SavePrefs(self.prefs.clone()));
                        out.notes.push(Note::Other);
                    }
                }
            }

            Command::CopyToolOutput(item) => out.merge(self.tool_output(item, OutputUse::Copy)),
            Command::SaveToolOutput(item) => out.merge(self.tool_output(item, OutputUse::Save)),
            Command::AnswerUiRequest { id, answer } => {
                if let Some(conv) = self.selected {
                    let c = self.conv_mut(conv).unwrap();
                    if c.ui_requests.iter().any(|r| r.id == id) && !c.ui_answering.contains(&id) {
                        c.ui_answering.push(id.clone());
                        c.ui_error = None;
                        let generation = c.generation;
                        out.effects.push(Effect::Backend(BackendRequest::UiRespond {
                            conversation: conv,
                            generation,
                            id,
                            answer,
                        }));
                        out.notes.push(Note::Other);
                    }
                }
            }
            Command::CancelUiRequest(id) => {
                if let Some(conv) = self.selected {
                    let c = self.conv_mut(conv).unwrap();
                    if c.ui_requests.iter().any(|r| r.id == id) {
                        let generation = c.generation;
                        out.effects.push(Effect::Backend(BackendRequest::UiCancel {
                            conversation: conv,
                            generation,
                            id,
                        }));
                    }
                }
            }
            Command::DismissUiNotices => {
                if let Some(conv) = self.selected {
                    let c = self.conv_mut(conv).unwrap();
                    if !c.ui_notices.is_empty() || c.ui_error.is_some() {
                        c.ui_notices.clear();
                        c.ui_error = None;
                        out.notes.push(Note::Other);
                    }
                }
            }
            Command::OpenSearchHit(i) => {
                if let Some(hit) = self.history_search.hits.get(i).cloned()
                    && self.conversation(hit.conversation).is_some()
                {
                    if self.selected != Some(hit.conversation) {
                        self.search_return = self.selected;
                    }
                    self.scroll_target = Some((hit.conversation, hit.item));
                    self.target_pages = 0;
                    // Showing the hit needs its conversation open, so a search never leaves the
                    // person looking at the wrong place.
                    self.search.clear();
                    self.history_search = SearchResults::default();
                    out.merge(self.dispatch(Command::SelectConversation(hit.conversation)));
                    out.merge(self.continue_to_target(hit.conversation));
                    out.notes.push(Note::ConversationsChanged);
                }
            }
            Command::ReturnFromSearch => {
                if let Some(back) = self.search_return.take() {
                    self.scroll_target = None;
                    out.merge(self.dispatch(Command::SelectConversation(back)));
                }
            }
            Command::ClearScrollTarget => {
                if self.scroll_target.take().is_some() {
                    out.notes.push(Note::Other);
                }
            }
            Command::OpenInEditor(i) => {
                if self.availability().open_in_editor
                    && let Some(project) = self.current_project()
                    && let Some(change) = self.current().and_then(|c| c.changes.get(i))
                {
                    let path = format!("{}/{}", project.path.trim_end_matches('/'), change.path);
                    out.effects.push(Effect::Launch(Launch::Editor {
                        path,
                        root: project.path.clone(),
                    }));
                }
            }
            Command::OpenTerminal => {
                if self.availability().open_terminal
                    && let Some(project) = self.current_project()
                {
                    out.effects.push(Effect::Launch(Launch::Terminal {
                        cwd: project.path.clone(),
                    }));
                }
            }

            Command::Submit => {
                if self.availability().submit {
                    out.merge(self.submit_from_draft(IntentOrigin::Draft));
                }
            }
            Command::Steer => {
                if self.availability().steer {
                    if self.mode == Mode::Real {
                        out.merge(self.submit_from_draft(IntentOrigin::Steer));
                    } else {
                        out.merge(self.steer());
                    }
                }
            }
            Command::QueueFollowUp => {
                if self.availability().queue && self.mode == Mode::Real {
                    out.merge(self.submit_from_draft(IntentOrigin::FollowUp));
                } else if self.availability().queue {
                    let id = self.selected.unwrap();
                    let qid = QueueId(self.next_queue);
                    self.next_queue += 1;
                    let c = self.conv_mut(id).unwrap();
                    let text = std::mem::take(&mut c.draft.text);
                    c.draft.attachments.clear();
                    c.queue.push(QueuedPrompt {
                        id: qid,
                        text,
                        mode: QueueMode::FollowUp,
                    });
                    out.merge(self.reset_draft(id));
                }
            }
            Command::RemoveQueued(q) => {
                if let Some(id) = self.selected {
                    let real = self.mode == Mode::Real;
                    let c = self.conv_mut(id).unwrap();
                    if real {
                        // The engine owns the queue: ask it, and let its answer remove the row.
                        if c.opened && c.queue.iter().any(|x| x.id == q) {
                            let generation = c.generation;
                            out.effects
                                .push(Effect::Backend(BackendRequest::CancelQueued {
                                    conversation: id,
                                    generation,
                                    entry: q,
                                }));
                        }
                    } else {
                        c.queue.retain(|x| x.id != q);
                        out.notes.push(Note::Other);
                    }
                }
            }
            Command::DismissStorageIssue => {
                if self.storage_issue.take().is_some() {
                    out.notes.push(Note::Other);
                }
            }
            Command::RefreshModels => {
                if self.availability().refresh_models {
                    let id = self.selected.unwrap();
                    let generation = self.conv(id).generation;
                    out.effects
                        .push(Effect::Backend(BackendRequest::RefreshModels {
                            conversation: id,
                            generation,
                        }));
                }
            }
            Command::RetrySave => {
                if let Some(id) = self.selected
                    && matches!(self.conv(id).draft.save, SaveState::Failed(_))
                {
                    self.conv_mut(id).unwrap().draft.save = SaveState::Dirty;
                    out.merge(self.flush_draft(id));
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
                            request: c.current_request.clone(),
                        }));
                }
            }
            Command::Retry => {
                if self.availability().retry {
                    let id = self.selected.unwrap();
                    let c = self.conv_mut(id).unwrap();
                    if let Some((text, atts)) = c.last_submission.clone() {
                        out.merge(self.submit(id, text, atts, IntentOrigin::Retry));
                    }
                }
            }
            Command::DismissFailure => {
                if let Some(id) = self.selected {
                    let c = self.conv_mut(id).unwrap();
                    if c.intent_error.take().is_some() {
                        out.notes.push(Note::Other);
                    }
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
                    c.older_error = None;
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

    /// Copy or save a tool call's complete output: at once when the preview holds all of it,
    /// otherwise by fetching it from the engine.
    fn tool_output(&mut self, item: ItemId, purpose: OutputUse) -> Outcome {
        let mut out = Outcome::default();
        let Some(conv) = self.selected else {
            return out;
        };
        let c = self.conv_mut(conv).unwrap();
        let Some(tool) = c.items.iter().find_map(|i| match &i.kind {
            ItemKind::Tool(t) if i.id == item => Some(t.clone()),
            _ => None,
        }) else {
            return out;
        };
        let name = format!("{}-output.txt", tool.name);
        let finish = |text: String, out: &mut Outcome| {
            out.effects.push(match purpose {
                OutputUse::Copy => Effect::CopyText(text),
                OutputUse::Save => Effect::SaveText {
                    suggested_name: name.clone(),
                    text,
                },
            });
        };
        match tool.call_id {
            // The preview is everything there is, or the engine cannot be asked about it.
            Some(_) if !tool.truncated => finish(tool.output, &mut out),
            None => finish(tool.output, &mut out),
            Some(call_id) => {
                c.pending_output = Some((call_id.clone(), purpose));
                let generation = c.generation;
                out.effects
                    .push(Effect::Backend(BackendRequest::FetchToolOutput {
                        conversation: conv,
                        generation,
                        call_id,
                    }));
            }
        }
        out
    }

    /// While a search hit's message is not among the loaded items, keep loading older history,
    /// up to a bound, and give up with a notice when it is not there.
    fn continue_to_target(&mut self, id: ConversationId) -> Outcome {
        let mut out = Outcome::default();
        let Some((target_conv, item)) = self.scroll_target else {
            return out;
        };
        if target_conv != id {
            return out;
        }
        let Some(c) = self.conversation(id) else {
            return out;
        };
        if !c.opened || c.items.iter().any(|i| i.id == item) {
            return out;
        }
        if c.has_older && !c.loading_older && self.target_pages < 60 {
            self.target_pages += 1;
            out.merge(self.dispatch(Command::LoadOlder));
        } else if !c.has_older || self.target_pages >= 60 {
            self.scroll_target = None;
            out.merge(
                self.set_notice("That message is no longer in this conversation's history.".into()),
            );
        }
        out
    }

    /// A conversation known only from its saved copy, so reading is possible before (or
    /// without) the engine's own list.
    pub fn restore_cached_conversation(
        &mut self,
        id: ConversationId,
        project_path: &str,
        title: String,
        updated_at: i64,
    ) {
        if self.conversation(id).is_some() || !project_path.starts_with('/') {
            return;
        }
        self.restore_project(project_path);
        let mut c =
            ConversationState::new(id, project_id_for_path(project_path), title, updated_at);
        c.cached_only = true;
        self.next_conversation = self.next_conversation.max(id.0 + 1);
        self.conversations.push(c);
    }

    /// Show a saved copy of a conversation until the engine's own state arrives. Ignored once
    /// the conversation has anything of its own to show.
    pub fn apply_cache(
        &mut self,
        id: ConversationId,
        items: Vec<TranscriptItem>,
        has_older: bool,
        synced_at: i64,
    ) -> Outcome {
        let mut out = Outcome::default();
        let Some(c) = self.conv_mut(id) else {
            return out;
        };
        // The engine's refusal to open can arrive before the saved copy is read; the copy is
        // still better than the notice, and keeps the reason it is shown.
        let refusal = match c.items.as_slice() {
            [
                TranscriptItem {
                    id: ItemId(i),
                    kind:
                        ItemKind::Notice {
                            text,
                            level: NoticeLevel::Error,
                        },
                    ..
                },
            ] if *i == LOCAL_ITEM_BASE - 1 => Some(text.clone()),
            _ => None,
        };
        if c.opened || (!c.items.is_empty() && refusal.is_none()) || items.is_empty() {
            return out;
        }
        if refusal.is_some() {
            c.stale_reason = refusal;
        }
        c.items = items;
        c.has_older = has_older;
        c.cached_at = Some(synced_at);
        out.notes.push(Note::ItemsReset(id));
        out
    }

    /// The controller's answer to `Effect::SearchHistory`. A result for text no longer in the
    /// search box is dropped.
    pub fn apply_search_results(&mut self, results: SearchResults) -> Outcome {
        let mut out = Outcome::default();
        if results.query != self.search.trim() {
            return out;
        }
        self.history_search = results;
        out.notes.push(Note::ConversationsChanged);
        out
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
                attachments: c.draft.attachments.clone(),
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

    fn submit_from_draft(&mut self, origin: IntentOrigin) -> Outcome {
        let id = self.selected.unwrap();
        let c = self.conv_mut(id).unwrap();
        let text = c.draft.text.trim_end().to_string();
        let atts = c.draft.attachments.clone();
        self.submit(id, text, atts, origin)
    }

    /// First half of a submission: record the intent. The draft stays and nothing is sent until
    /// `intent_persisted` reports the commit.
    fn submit(
        &mut self,
        id: ConversationId,
        text: String,
        atts: Vec<Attachment>,
        origin: IntentOrigin,
    ) -> Outcome {
        let mut out = Outcome::default();
        if self.conv_mut(id).unwrap().pending_intent.is_some() {
            return out;
        }
        let request = self.alloc_request();
        let model = self.prefs.model.clone();
        let c = self.conv_mut(id).unwrap();
        c.intent_error = None;
        c.pending_intent = Some(PendingIntent {
            request: request.clone(),
            origin,
            text: text.clone(),
            attachments: atts.clone(),
            model: model.clone(),
        });
        out.effects.push(Effect::JournalIntent {
            conversation: id,
            request,
            text,
            attachments: atts,
            model,
        });
        out.notes.push(Note::Other);
        out
    }

    /// The journal reported the outcome of a `JournalIntent`. On success the draft is cleared
    /// (if the user has not edited it since) and the request is sent; on failure the draft is
    /// kept and nothing is sent.
    pub fn intent_persisted(
        &mut self,
        id: ConversationId,
        request: &RequestId,
        result: Result<(), String>,
    ) -> Outcome {
        let mut out = Outcome::default();
        let Some(c) = self.conv_mut(id) else {
            return out;
        };
        if c.pending_intent.as_ref().map(|p| &p.request) != Some(request) {
            return out; // stale or duplicate acknowledgment
        }
        let intent = c.pending_intent.take().unwrap();
        if let Err(e) = result {
            c.intent_error = Some(format!(
                "Could not save the prompt before sending, so it was not sent: {e}"
            ));
            if intent.origin == IntentOrigin::Queue {
                let qid = QueueId(self.next_queue);
                self.next_queue += 1;
                self.conv_mut(id).unwrap().queue.insert(
                    0,
                    QueuedPrompt {
                        id: qid,
                        text: intent.text,
                        mode: QueueMode::FollowUp,
                    },
                );
            }
            out.notes.push(Note::Other);
            return out;
        }
        out.merge(self.dispatch_intent(id, intent));
        out
    }

    /// Second half of a submission, after the intent is durable.
    fn dispatch_intent(&mut self, id: ConversationId, intent: PendingIntent) -> Outcome {
        let mut out = Outcome::default();
        let op = self.alloc_op();
        let now = self.now;
        let c = self.conv_mut(id).unwrap();
        let mut clear_draft = false;
        match intent.origin {
            IntentOrigin::Draft => {
                // Keep anything the user typed after pressing send.
                clear_draft = c.draft.text.trim_end() == intent.text
                    && c.draft
                        .attachments
                        .iter()
                        .map(|a| &a.path)
                        .eq(intent.attachments.iter().map(|a| &a.path));
                if clear_draft {
                    c.draft.text.clear();
                    c.draft.attachments.clear();
                }
            }
            IntentOrigin::Retry => {
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
            }
            IntentOrigin::Queue => {}
            IntentOrigin::Steer | IntentOrigin::FollowUp => {
                // The run is untouched: the engine owns this input from here, and shows it in
                // its queue and, once placed, in the transcript.
                let draft_text = c.draft.text.trim_end() == intent.text
                    && c.draft
                        .attachments
                        .iter()
                        .map(|a| &a.path)
                        .eq(intent.attachments.iter().map(|a| &a.path));
                if draft_text {
                    c.draft.text.clear();
                    c.draft.attachments.clear();
                }
                let mode = if intent.origin == IntentOrigin::Steer {
                    QueueMode::Steer
                } else {
                    QueueMode::FollowUp
                };
                c.pending_queue.push(PendingQueued {
                    request: intent.request.clone(),
                    text: intent.text.clone(),
                    attachments: intent.attachments.clone(),
                    mode,
                    state: QueueSend::Sending,
                });
                let generation = c.generation;
                out.effects.push(Effect::Backend(BackendRequest::Queue {
                    conversation: id,
                    generation,
                    request: intent.request,
                    mode,
                    text: intent.text,
                    attachments: intent.attachments,
                }));
                if draft_text {
                    out.merge(self.reset_draft(id));
                }
                out.notes.push(Note::Other);
                return out;
            }
        }
        c.run = RunState::Submitting { op };
        c.last_submission = Some((intent.text.clone(), intent.attachments.clone()));
        c.current_request = Some(intent.request.clone());
        c.updated_at = now;
        c.streaming_item = None;
        let generation = c.generation;
        self.push_item(
            id,
            ItemKind::User {
                text: intent.text.clone(),
                attachments: intent.attachments.clone(),
                delivery: Delivery::Pending,
                steer: false,
            },
            &mut out,
        );
        if clear_draft {
            out.merge(self.reset_draft(id));
        }
        out.effects.push(Effect::Backend(BackendRequest::Submit {
            conversation: id,
            generation,
            op,
            request: intent.request,
            text: intent.text,
            attachments: intent.attachments,
            model: intent.model,
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
        let real = self.mode == Mode::Real;
        // What is worth keeping a saved copy of: a settled view of the conversation.
        let cache = real
            && matches!(
                ev.kind,
                EventKind::Opened { .. }
                    | EventKind::OlderPage { .. }
                    | EventKind::Synced { .. }
                    | EventKind::Completed
                    | EventKind::Failed { .. }
                    | EventKind::Cancelled
            );
        let reached_target = matches!(
            ev.kind,
            EventKind::Opened { .. } | EventKind::OlderPage { .. }
        );
        let Some(c) = self.conv_mut(id) else {
            return out;
        };
        // Stale generation: an earlier attachment's content must never land here. What became of
        // a request or a run is not content: the operation or request key identifies it, so it
        // applies however many times the conversation was attached since.
        let by_identity = matches!(
            ev.kind,
            EventKind::Accepted
                | EventKind::Rejected { .. }
                | EventKind::AckLost
                | EventKind::StatusResolved { .. }
                | EventKind::Completed
                | EventKind::Failed { .. }
                | EventKind::Cancelled
                | EventKind::QueueAdmitted { .. }
                | EventKind::QueueRefused { .. }
                | EventKind::QueueAckLost { .. }
                | EventKind::QueueCancelled { .. }
                | EventKind::ToolOutputFull { .. }
                | EventKind::ToolOutputUnavailable { .. }
                | EventKind::UiRespondRefused { .. }
        );
        if ev.generation != c.generation && !by_identity {
            return out;
        }
        // Operation-scoped events must match the live operation.
        let scoped = !matches!(
            ev.kind,
            EventKind::Opened { .. }
                | EventKind::OlderPage { .. }
                | EventKind::Synced { .. }
                | EventKind::OpenFailed { .. }
                | EventKind::ChangesSynced(_)
                | EventKind::QueueAdmitted { .. }
                | EventKind::QueueRefused { .. }
                | EventKind::QueueAckLost { .. }
                | EventKind::QueueCancelled { .. }
                | EventKind::EngineState { .. }
                | EventKind::OlderFailed { .. }
                | EventKind::HasOlder(_)
                | EventKind::ToolOutputFull { .. }
                | EventKind::ToolOutputUnavailable { .. }
                | EventKind::UiState { .. }
                | EventKind::UiRespondRefused { .. }
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
                c.streaming_item = None;
                c.has_older = has_older;
                c.opened = true;
                c.cached_at = None;
                c.stale_reason = None;
                c.selected_change = (!changes.is_empty()).then_some(0);
                c.changes = changes;
                out.notes.push(Note::ItemsReset(id));
                // A submission in doubt (restored from the journal, or cut off by a reconnect)
                // is resolved by asking the engine about its key; that never sends anything.
                if let RunState::OutcomeUnknown { op } = c.run {
                    out.effects
                        .push(Effect::Backend(BackendRequest::CheckStatus {
                            conversation: id,
                            generation: c.generation,
                            op,
                            request: c.current_request.clone(),
                        }));
                }
            }
            EventKind::OpenFailed { message } if c.cached_at.is_some() => {
                // The saved copy stays, labelled with why the engine's own state is missing.
                c.stale_reason = Some(message);
                out.notes.push(Note::Other);
            }
            EventKind::OpenFailed { message } => {
                if !c.opened {
                    c.items = vec![TranscriptItem {
                        id: ItemId(LOCAL_ITEM_BASE - 1),
                        at: 0,
                        kind: ItemKind::Notice {
                            text: message,
                            level: NoticeLevel::Error,
                        },
                    }];
                    out.notes.push(Note::ItemsReset(id));
                }
            }
            EventKind::Synced { items } => {
                if c.opened {
                    c.items = items;
                    c.streaming_item = None;
                    out.notes.push(Note::ItemsReset(id));
                }
            }
            EventKind::OlderPage { items, has_older } => {
                let n = items.len();
                c.loading_older = false;
                c.older_error = None;
                c.has_older = has_older;
                let mut v = items;
                v.append(&mut c.items);
                c.items = v;
                if let Some(s) = c.streaming_item.as_mut() {
                    *s += n;
                }
                // A tool call and its result can fall on either side of the page boundary.
                if merge_boundary_tools(&mut c.items, n) {
                    c.streaming_item = None;
                    out.notes.push(Note::ItemsReset(id));
                } else {
                    out.notes.push(Note::ItemsPrepended(id, n));
                }
            }
            EventKind::HasOlder(has) => {
                if c.opened && c.has_older != has {
                    c.has_older = has;
                    out.notes.push(Note::Other);
                }
            }
            EventKind::OlderFailed { message } => {
                c.loading_older = false;
                c.older_error = Some(message);
                out.notes.push(Note::Other);
            }
            EventKind::ToolOutputFull { call_id, text } => {
                if let Some((wanted, purpose)) = c.pending_output.clone()
                    && wanted == call_id
                {
                    c.pending_output = None;
                    let name = c
                        .items
                        .iter()
                        .find_map(|i| match &i.kind {
                            ItemKind::Tool(t) if t.call_id.as_deref() == Some(call_id.as_str()) => {
                                Some(format!("{}-output.txt", t.name))
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| "tool-output.txt".into());
                    out.effects.push(match purpose {
                        OutputUse::Copy => Effect::CopyText(text),
                        OutputUse::Save => Effect::SaveText {
                            suggested_name: name,
                            text,
                        },
                    });
                }
            }
            EventKind::ToolOutputUnavailable { call_id, reason } => {
                if c.pending_output
                    .as_ref()
                    .is_some_and(|(w, _)| *w == call_id)
                {
                    c.pending_output = None;
                    out.merge(
                        self.set_notice(format!("Could not get the complete output: {reason}")),
                    );
                }
            }
            EventKind::UiState {
                requests,
                status,
                notices,
            } => {
                c.ui_answering
                    .retain(|a| requests.iter().any(|r| &r.id == a));
                c.ui_requests = requests;
                c.ui_status = status;
                c.ui_notices = notices;
                out.notes.push(Note::Other);
            }
            EventKind::UiRespondRefused {
                id: request,
                reason,
            } => {
                c.ui_answering.retain(|a| *a != request);
                c.ui_error = Some(reason);
                out.notes.push(Note::Other);
            }
            EventKind::Accepted => {
                if let RunState::Submitting { op } = c.run {
                    c.run = RunState::Running { op };
                    mark_last_user(c, Delivery::Sent, &mut out, id);
                    journal(c, id, JournalState::Accepted, &mut out);
                }
            }
            EventKind::SteerAccepted => {
                mark_last_user(c, Delivery::Sent, &mut out, id);
            }
            EventKind::Rejected { reason } => {
                if let RunState::Submitting { .. } = c.run {
                    journal(c, id, JournalState::Rejected, &mut out);
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
                    journal(c, id, JournalState::Unknown, &mut out);
                    mark_last_user(c, Delivery::Unknown, &mut out, id);
                    out.notes.push(Note::Other);
                }
            }
            EventKind::StatusResolved { accepted } => {
                if let RunState::OutcomeUnknown { op } = c.run {
                    if accepted {
                        c.run = RunState::Running { op };
                        journal(c, id, JournalState::Accepted, &mut out);
                        mark_last_user(c, Delivery::Sent, &mut out, id);
                    } else {
                        journal(c, id, JournalState::Rejected, &mut out);
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
                        call_id: None,
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
            EventKind::ChangesSynced(changes) => {
                if c.opened {
                    c.selected_change = c
                        .selected_change
                        .filter(|i| *i < changes.len())
                        .or((!changes.is_empty()).then_some(0));
                    c.changes = changes;
                    out.notes.push(Note::Other);
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
            EventKind::QueueAdmitted { request, entry } => {
                let pending = c.pending_queue.iter().position(|p| p.request == request);
                if let Some(i) = pending {
                    let p = c.pending_queue.remove(i);
                    if c.queue.iter().all(|q| q.id != entry) {
                        c.queue.push(QueuedPrompt {
                            id: entry,
                            text: p.text,
                            mode: p.mode,
                        });
                    }
                    out.effects.push(Effect::JournalState {
                        conversation: id,
                        request,
                        state: JournalState::Queued,
                    });
                    out.notes.push(Note::Other);
                }
            }
            EventKind::QueueRefused { request, reason } => {
                if let Some(i) = c.pending_queue.iter().position(|p| p.request == request) {
                    let p = c.pending_queue.remove(i);
                    out.effects.push(Effect::JournalState {
                        conversation: id,
                        request,
                        state: JournalState::Rejected,
                    });
                    // The text comes back to the draft if the user has not started another.
                    if c.draft.text.is_empty() && c.draft.attachments.is_empty() {
                        c.draft.text = p.text;
                        c.draft.attachments = p.attachments;
                        c.draft.sync_epoch += 1;
                        c.draft.rev += 1;
                        c.draft.save = SaveState::Dirty;
                    }
                    c.intent_error = Some(format!("Not queued: {reason}"));
                    out.notes.push(Note::Other);
                }
            }
            EventKind::QueueAckLost { request } => {
                if let Some(p) = c.pending_queue.iter_mut().find(|p| p.request == request) {
                    p.state = QueueSend::Unknown;
                    out.effects.push(Effect::JournalState {
                        conversation: id,
                        request,
                        state: JournalState::Unknown,
                    });
                    out.notes.push(Note::Other);
                }
            }
            EventKind::QueueCancelled { entry, outcome } => {
                match outcome {
                    CancelOutcome::Cancelled | CancelOutcome::NotFound => {
                        c.queue.retain(|q| q.id != entry);
                    }
                    CancelOutcome::AlreadyConsumed => {
                        c.intent_error = Some(
                            "That input had already started, so it could not be removed.".into(),
                        );
                    }
                }
                out.notes.push(Note::Other);
            }
            EventKind::EngineState { busy, queue } => {
                if real {
                    c.queue = queue;
                    match (busy, c.run.clone()) {
                        // Work this window did not start: show it and let the engine end it.
                        (true, RunState::Idle) => {
                            let op = self.alloc_op();
                            let c = self.conv_mut(id).unwrap();
                            c.run = RunState::Running { op };
                            c.adopted = Some(op);
                        }
                        (false, RunState::Running { op } | RunState::Stopping { op })
                            if c.adopted == Some(op) =>
                        {
                            c.run = RunState::Idle;
                            c.adopted = None;
                        }
                        _ => {}
                    }
                    out.notes.push(Note::Other);
                }
            }
            EventKind::Completed => {
                c.run = RunState::Idle;
                journal(c, id, JournalState::Completed, &mut out);
                self.finish_streaming(id, &mut out);
                out.merge(self.after_settled(id));
            }
            EventKind::Failed { message } => {
                c.run = RunState::Failed {
                    message: message.clone(),
                };
                journal(c, id, JournalState::Failed, &mut out);
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
                    journal(c, id, JournalState::Cancelled, &mut out);
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
        if cache && self.conversation(id).is_some_and(|c| c.opened) {
            out.effects.push(Effect::SaveCache { conversation: id });
        }
        if reached_target {
            out.merge(self.continue_to_target(id));
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

    /// After a run completes normally, start the next queued prompt. Only the demo keeps its own
    /// queue: a real engine consumes the queue it owns, and starting one of its entries from
    /// here too would run it twice.
    fn after_settled(&mut self, id: ConversationId) -> Outcome {
        if self.mode == Mode::Real {
            return Outcome::default();
        }
        let c = self.conv_mut(id).unwrap();
        if c.queue.is_empty() {
            return Outcome::default();
        }
        let next = c.queue.remove(0);
        self.submit(id, next.text, vec![], IntentOrigin::Queue)
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

    /// Restore a draft (text and attachments) read from storage at startup.
    pub fn restore_draft(
        &mut self,
        id: ConversationId,
        text: String,
        attachments: Vec<Attachment>,
    ) {
        if let Some(c) = self.conv_mut(id)
            && (!text.is_empty() || !attachments.is_empty())
            && c.draft.text.is_empty()
            && c.draft.attachments.is_empty()
        {
            c.draft.text = text;
            c.draft.attachments = attachments;
            c.draft.sync_epoch += 1;
            c.draft.save = SaveState::Saved;
        }
    }

    /// Restore a journaled request that never reached a terminal state before the last exit.
    /// Whether the engine accepted it is unknown, so the conversation enters `OutcomeUnknown`
    /// and the user decides (check status, then retry). Nothing is resent here. If the
    /// conversation already has an older unresolved request, that one is superseded.
    pub fn restore_unresolved(
        &mut self,
        id: ConversationId,
        request: RequestId,
        text: String,
        attachments: Vec<Attachment>,
    ) -> Outcome {
        let mut out = Outcome::default();
        if self.conv_mut(id).is_none() {
            return out;
        }
        let op = self.alloc_op();
        let c = self.conv_mut(id).unwrap();
        if let Some(previous) = c.current_request.replace(request) {
            out.effects.push(Effect::JournalState {
                conversation: id,
                request: previous,
                state: JournalState::Superseded,
            });
        }
        c.run = RunState::OutcomeUnknown { op };
        c.last_submission = Some((text, attachments));
        out.notes.push(Note::Other);
        out
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

/// A page of older history can end on a tool call whose result is the first thing already
/// shown (or the other way round). Join the two so one call is one item. Returns whether
/// anything was joined.
fn merge_boundary_tools(items: &mut Vec<TranscriptItem>, new: usize) -> bool {
    let mut merged = false;
    let mut i = 0;
    while i < new.min(items.len()) {
        let call = match &items[i].kind {
            ItemKind::Tool(t) if t.status == ToolStatus::Running && t.output.is_empty() => {
                t.call_id.clone()
            }
            _ => None,
        };
        if let Some(call_id) = call {
            let found = (new..items.len().min(new + 8)).find(|j| {
                matches!(&items[*j].kind, ItemKind::Tool(t)
                    if t.call_id.as_deref() == Some(call_id.as_str()) && t.status != ToolStatus::Running)
            });
            if let Some(j) = found {
                let ItemKind::Tool(done) = items.remove(j).kind else {
                    unreachable!()
                };
                if let ItemKind::Tool(t) = &mut items[i].kind {
                    t.output = done.output;
                    t.truncated = done.truncated;
                    t.full_len = done.full_len;
                    t.status = done.status;
                    if t.input.is_empty() {
                        t.input = done.input;
                    }
                }
                merged = true;
            }
        }
        i += 1;
    }
    merged
}

/// Record a journal transition for the conversation's current request. Terminal states end
/// the association, so a later event can never rewrite a settled request.
fn journal(c: &mut ConversationState, id: ConversationId, state: JournalState, out: &mut Outcome) {
    let Some(request) = c.current_request.clone() else {
        return;
    };
    if state.is_terminal() {
        c.current_request = None;
    }
    out.effects.push(Effect::JournalState {
        conversation: id,
        request,
        state,
    });
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

/// Backend projects plus the folders the user opened, without duplicates (by id).
fn merge_projects(mut from_backend: Vec<Project>, bookmarks: &[Project]) -> Vec<Project> {
    for bookmark in bookmarks {
        if from_backend.iter().all(|p| p.id != bookmark.id) {
            from_backend.push(bookmark.clone());
        }
    }
    from_backend
}

/// Bound a tool output to the preview size, never splitting a character. Returns the preview
/// and whether anything was cut.
pub fn preview_output(text: &str) -> (String, bool) {
    if text.len() <= TOOL_OUTPUT_PREVIEW_BYTES {
        return (text.to_owned(), false);
    }
    let cut = floor_boundary(text, TOOL_OUTPUT_PREVIEW_BYTES);
    (text[..cut].to_owned(), true)
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
