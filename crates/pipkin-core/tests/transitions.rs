use pipkin_core::*;

fn state() -> AppState {
    let boot = Bootstrap {
        projects: vec![Project {
            id: ProjectId(1),
            name: "demo".into(),
            path: "/demo".into(),
        }],
        models: vec![
            ModelInfo {
                id: "a".into(),
                name: "A".into(),
                note: String::new(),
            },
            ModelInfo {
                id: "b".into(),
                name: "B".into(),
                note: String::new(),
            },
        ],
        conversations: vec![
            (ConversationId(1), ProjectId(1), "one".into(), 10),
            (ConversationId(2), ProjectId(1), "two".into(), 5),
        ],
        now: 100,
    };
    let mut s = AppState::new(boot, Prefs::default());
    s.prefs.selected_project = Some(ProjectId(1));
    s.dispatch(Command::SelectConversation(ConversationId(1)));
    s
}

fn ev(s: &AppState, kind: EventKind) -> BackendEvent {
    let c = s.current().unwrap();
    BackendEvent {
        conversation: c.id,
        generation: c.generation,
        op: c.run.op(),
        kind,
    }
}

/// Commit every journal intent in `out`, as the controller does once storage acknowledges.
fn settle(s: &mut AppState, mut out: Outcome) -> Outcome {
    let intents: Vec<_> = out
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::JournalIntent {
                conversation,
                request,
                ..
            } => Some((*conversation, request.clone())),
            _ => None,
        })
        .collect();
    for (conversation, request) in intents {
        out.merge(s.intent_persisted(conversation, &request, Ok(())));
    }
    out
}

fn send(s: &mut AppState, command: Command) -> Outcome {
    let out = s.dispatch(command);
    settle(s, out)
}

fn apply(s: &mut AppState, event: BackendEvent) -> Outcome {
    let out = s.apply_event(event);
    settle(s, out)
}

fn run_started(s: &mut AppState, text: &str) -> OperationId {
    s.dispatch(Command::EditDraft(text.into()));
    send(s, Command::Submit);
    let op = s.current().unwrap().run.op().unwrap();
    let e = ev(s, EventKind::Accepted);
    s.apply_event(e);
    op
}

#[test]
fn submit_moves_draft_into_transcript_and_runs() {
    let mut s = state();
    s.dispatch(Command::EditDraft("fix the test".into()));
    let out = send(&mut s, Command::Submit);
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Backend(BackendRequest::Submit { text, .. }) if text == "fix the test")));
    let c = s.current().unwrap();
    assert_eq!(c.draft.text, "");
    assert!(matches!(c.run, RunState::Submitting { .. }));
    assert!(matches!(
        &c.items.last().unwrap().kind,
        ItemKind::User {
            delivery: Delivery::Pending,
            ..
        }
    ));
    let e = ev(&s, EventKind::Accepted);
    s.apply_event(e);
    assert!(matches!(s.current().unwrap().run, RunState::Running { .. }));
}

#[test]
fn cannot_submit_empty_or_twice() {
    let mut s = state();
    assert!(!s.availability().submit);
    run_started(&mut s, "go");
    s.dispatch(Command::EditDraft("again".into()));
    assert!(!s.availability().submit);
    assert!(s.availability().steer && s.availability().queue);
}

#[test]
fn stale_generation_and_op_events_are_dropped() {
    let mut s = state();
    let op = run_started(&mut s, "go");
    let (cid, cgen) = {
        let c = s.current().unwrap();
        (c.id, c.generation)
    };
    let stale_gen = BackendEvent {
        conversation: cid,
        generation: cgen + 5,
        op: Some(op),
        kind: EventKind::Token("x".into()),
    };
    s.apply_event(stale_gen);
    let wrong_op = BackendEvent {
        conversation: cid,
        generation: cgen,
        op: Some(OperationId(999)),
        kind: EventKind::Token("x".into()),
    };
    s.apply_event(wrong_op);
    assert!(
        !s.current()
            .unwrap()
            .items
            .iter()
            .any(|i| matches!(i.kind, ItemKind::Assistant { .. }))
    );
}

#[test]
fn late_event_lands_in_its_own_conversation_not_selected_one() {
    let mut s = state();
    let op = run_started(&mut s, "go");
    s.dispatch(Command::SelectConversation(ConversationId(2)));
    s.apply_event(BackendEvent {
        conversation: ConversationId(1),
        generation: 1,
        op: Some(op),
        kind: EventKind::Token("hi".into()),
    });
    assert!(s.conversation(ConversationId(2)).unwrap().items.is_empty());
    assert_eq!(s.conversation(ConversationId(1)).unwrap().items.len(), 2);
}

#[test]
fn cancel_waits_for_settlement() {
    let mut s = state();
    run_started(&mut s, "go");
    s.dispatch(Command::Cancel);
    assert!(matches!(
        s.current().unwrap().run,
        RunState::Stopping { .. }
    ));
    // Completion racing the cancel must not be treated as settled-by-us; stays until event.
    assert!(!s.availability().cancel);
    let e = ev(&s, EventKind::Cancelled);
    s.apply_event(e);
    assert_eq!(s.current().unwrap().run, RunState::Idle);
}

#[test]
fn unknown_outcome_never_resends() {
    let mut s = state();
    s.dispatch(Command::EditDraft("risky".into()));
    send(&mut s, Command::Submit);
    let e = ev(&s, EventKind::AckLost);
    s.apply_event(e);
    assert!(matches!(
        s.current().unwrap().run,
        RunState::OutcomeUnknown { .. }
    ));
    s.dispatch(Command::EditDraft("more".into()));
    assert!(!s.availability().submit && !s.availability().retry);
    let out = s.dispatch(Command::Submit);
    assert!(out.effects.is_empty());
    let out = s.dispatch(Command::CheckStatus);
    assert!(matches!(
        out.effects[0],
        Effect::Backend(BackendRequest::CheckStatus { .. })
    ));
    let e = ev(&s, EventKind::StatusResolved { accepted: true });
    s.apply_event(e);
    assert!(matches!(s.current().unwrap().run, RunState::Running { .. }));
}

#[test]
fn rejection_retains_text_and_allows_explicit_retry() {
    let mut s = state();
    s.dispatch(Command::EditDraft("keep me".into()));
    send(&mut s, Command::Submit);
    let e = ev(
        &s,
        EventKind::Rejected {
            reason: "provider unavailable".into(),
        },
    );
    s.apply_event(e);
    let c = s.current().unwrap();
    assert_eq!(c.draft.text, "keep me");
    assert!(matches!(c.run, RunState::Failed { .. }));
    assert!(s.availability().retry);
    let out = send(&mut s, Command::Retry);
    assert!(
        out.effects
            .iter()
            .any(|e| matches!(e, Effect::Backend(BackendRequest::Submit { .. })))
    );
}

#[test]
fn queue_runs_next_after_completion_but_not_after_cancel() {
    let mut s = state();
    run_started(&mut s, "first");
    for t in ["second", "third"] {
        s.dispatch(Command::EditDraft(t.into()));
        s.dispatch(Command::QueueFollowUp);
    }
    assert_eq!(s.current().unwrap().queue.len(), 2);
    let e = ev(&s, EventKind::Completed);
    let out = apply(&mut s, e);
    assert!(out.effects.iter().any(
        |e| matches!(e, Effect::Backend(BackendRequest::Submit { text, .. }) if text == "second")
    ));
    assert_eq!(s.current().unwrap().queue.len(), 1);
    s.dispatch(Command::Cancel);
    let e = ev(&s, EventKind::Cancelled);
    let out = s.apply_event(e);
    assert!(
        !out.effects
            .iter()
            .any(|e| matches!(e, Effect::Backend(BackendRequest::Submit { .. })))
    );
    assert_eq!(s.current().unwrap().queue.len(), 1);
    let q = s.current().unwrap().queue[0].id;
    s.dispatch(Command::RemoveQueued(q));
    assert!(s.current().unwrap().queue.is_empty());
}

#[test]
fn tool_output_is_bounded() {
    let mut s = state();
    run_started(&mut s, "go");
    let e = ev(
        &s,
        EventKind::ToolStarted {
            call: 1,
            name: "bash".into(),
            input: "cargo test".into(),
        },
    );
    s.apply_event(e);
    let big = "é".repeat(20_000);
    let e = ev(
        &s,
        EventKind::ToolOutput {
            call: 1,
            chunk: big,
        },
    );
    s.apply_event(e);
    let e = ev(&s, EventKind::ToolFinished { call: 1, ok: false });
    s.apply_event(e);
    let ItemKind::Tool(t) = &s.current().unwrap().items.last().unwrap().kind else {
        panic!()
    };
    assert!(t.truncated && t.output.len() <= TOOL_OUTPUT_PREVIEW_BYTES && t.full_len == 40_000);
    assert_eq!(t.status, ToolStatus::Failed);
}

#[test]
fn drafts_are_per_conversation_and_flushed_on_switch() {
    let mut s = state();
    s.dispatch(Command::EditDraft("draft one".into()));
    let out = s.dispatch(Command::SelectConversation(ConversationId(2)));
    assert!(out.effects.iter().any(|e| matches!(e, Effect::SaveDraft { conversation, text, .. } if *conversation == ConversationId(1) && text == "draft one")));
    assert_eq!(s.current().unwrap().draft.text, "");
    s.dispatch(Command::SelectConversation(ConversationId(1)));
    assert_eq!(s.current().unwrap().draft.text, "draft one");
}

#[test]
fn save_acknowledgement_and_failure() {
    let mut s = state();
    s.dispatch(Command::EditDraft("x".into()));
    let out = s.dispatch(Command::FlushDraft(ConversationId(1)));
    let Effect::SaveDraft { rev, .. } = out.effects[0] else {
        panic!()
    };
    assert_eq!(s.current().unwrap().draft.save, SaveState::Saving);
    // Edit during the write: acknowledged rev is stale, stays Dirty.
    s.dispatch(Command::EditDraft("xy".into()));
    s.draft_saved(ConversationId(1), rev, Ok(()));
    assert_eq!(s.current().unwrap().draft.save, SaveState::Dirty);
    let out = s.dispatch(Command::FlushDraft(ConversationId(1)));
    let Effect::SaveDraft { rev, .. } = out.effects[0] else {
        panic!()
    };
    s.draft_saved(ConversationId(1), rev, Err("disk full".into()));
    let c = s.current().unwrap();
    assert!(matches!(c.draft.save, SaveState::Failed(_)));
    assert_eq!(c.draft.text, "xy");
}

#[test]
fn history_prepend_reports_count_and_shifts_streaming_index() {
    let mut s = state();
    run_started(&mut s, "go");
    let e = ev(&s, EventKind::Token("a".into()));
    s.apply_event(e);
    let page: Vec<_> = (0..5)
        .map(|i| TranscriptItem {
            id: ItemId(i),
            at: 0,
            kind: ItemKind::Notice {
                text: "old".into(),
                level: NoticeLevel::Info,
            },
        })
        .collect();
    let (cid, cgen) = {
        let c = s.current().unwrap();
        (c.id, c.generation)
    };
    let out = s.apply_event(BackendEvent {
        conversation: cid,
        generation: cgen,
        op: None,
        kind: EventKind::OlderPage {
            items: page,
            has_older: false,
        },
    });
    assert!(out.notes.contains(&Note::ItemsPrepended(cid, 5)));
    let e = ev(&s, EventKind::Token("b".into()));
    s.apply_event(e);
    let ItemKind::Assistant { text, .. } = &s.current().unwrap().items.last().unwrap().kind else {
        panic!()
    };
    assert_eq!(text, "ab");
}

#[test]
fn search_filters_and_new_conversation_selects() {
    let mut s = state();
    s.dispatch(Command::SetSearch("TWO".into()));
    assert_eq!(s.visible_conversations().len(), 1);
    s.dispatch(Command::SetSearch("zzz".into()));
    assert!(s.visible_conversations().is_empty());
    s.dispatch(Command::SetSearch(String::new()));
    s.dispatch(Command::NewConversation);
    assert_eq!(s.current().unwrap().title, "New conversation");
    s.dispatch(Command::RenameConversation(
        s.selected.unwrap(),
        "  Renamed ".into(),
    ));
    assert_eq!(s.current().unwrap().title, "Renamed");
}

#[test]
fn nothing_reaches_the_backend_until_connected() {
    let mut s = state();
    s.dispatch(Command::EditDraft("hello".into()));
    assert!(s.availability().submit);
    for c in [
        Connection::Connecting,
        Connection::Reconnecting,
        Connection::Offline("x".into()),
        Connection::Incompatible("x".into()),
        Connection::Failed("x".into()),
    ] {
        s.set_connection(c);
        assert_eq!(s.availability(), Availability::default());
        assert!(s.dispatch(Command::Submit).effects.is_empty());
    }
    s.set_connection(Connection::Ready);
    assert!(s.availability().submit);
}

#[test]
fn real_mode_does_not_invent_conversations() {
    let mut s = state();
    s.mode = Mode::Real;
    let before = s.conversations.len();
    let out = s.dispatch(Command::NewConversation);
    assert!(out.effects.is_empty() && out.notes.is_empty());
    assert_eq!(s.conversations.len(), before);
}

fn empty_state(prefs: Prefs) -> AppState {
    AppState::new(
        Bootstrap {
            projects: vec![],
            models: vec![],
            conversations: vec![],
            now: 1,
        },
        prefs,
    )
}

fn catalog() -> Bootstrap {
    Bootstrap {
        projects: vec![Project {
            id: ProjectId(1),
            name: "p".into(),
            path: "/p".into(),
        }],
        models: vec![ModelInfo {
            id: "m".into(),
            name: "M".into(),
            note: String::new(),
        }],
        conversations: vec![
            (ConversationId(1), ProjectId(1), "one".into(), 10),
            (ConversationId(2), ProjectId(1), "two".into(), 5),
        ],
        now: 50,
    }
}

#[test]
fn catalog_arrival_selects_and_opens_first_conversation() {
    let mut s = empty_state(Prefs::default());
    assert!(s.select_initial().effects.is_empty());
    s.apply_catalog(catalog());
    assert_eq!(s.prefs.model.as_deref(), Some("m"));
    let out = s.select_initial();
    assert_eq!(s.selected, Some(ConversationId(1)));
    assert!(out.effects.iter().any(|e| matches!(
        e,
        Effect::Backend(BackendRequest::Open { conversation, .. }) if *conversation == ConversationId(1)
    )));
}

#[test]
fn catalog_honors_remembered_selection_and_keeps_local_state() {
    let prefs = Prefs {
        selected_conversation: Some(ConversationId(2)),
        ..Prefs::default()
    };
    let mut s = empty_state(prefs);
    s.apply_catalog(catalog());
    s.restore_draft(ConversationId(2), "kept".into());
    s.apply_catalog(catalog());
    assert_eq!(s.conversations.len(), 2);
    s.select_initial();
    assert_eq!(s.selected, Some(ConversationId(2)));
    assert_eq!(s.current().unwrap().draft.text, "kept");
}

fn journal_states(out: &Outcome) -> Vec<JournalState> {
    out.effects
        .iter()
        .filter_map(|e| match e {
            Effect::JournalState { state, .. } => Some(*state),
            _ => None,
        })
        .collect()
}

fn backend_submits(out: &Outcome) -> usize {
    out.effects
        .iter()
        .filter(|e| matches!(e, Effect::Backend(BackendRequest::Submit { .. })))
        .count()
}

fn intent_request(out: &Outcome) -> RequestId {
    out.effects
        .iter()
        .find_map(|e| match e {
            Effect::JournalIntent { request, .. } => Some(request.clone()),
            _ => None,
        })
        .expect("a journal intent")
}

#[test]
fn nothing_is_sent_or_cleared_until_the_intent_is_durable() {
    let mut s = state();
    s.dispatch(Command::EditDraft("fix it".into()));
    let out = s.dispatch(Command::Submit);
    assert_eq!(backend_submits(&out), 0);
    let request = intent_request(&out);
    let c = s.current().unwrap();
    assert_eq!(c.draft.text, "fix it", "draft is kept until the commit");
    assert_eq!(c.run, RunState::Idle);
    assert!(c.items.is_empty());
    let a = s.availability();
    assert!(!a.submit && !a.retry, "no second submission while waiting");
    assert!(s.dispatch(Command::Submit).effects.is_empty());

    let out = s.intent_persisted(ConversationId(1), &request, Ok(()));
    assert_eq!(backend_submits(&out), 1);
    assert!(out.effects.iter().any(|e| matches!(
        e,
        Effect::Backend(BackendRequest::Submit { request: r, .. }) if *r == request
    )));
    let c = s.current().unwrap();
    assert_eq!(c.draft.text, "");
    assert!(matches!(c.run, RunState::Submitting { .. }));
}

#[test]
fn failed_journal_write_keeps_the_draft_and_sends_nothing() {
    let mut s = state();
    s.dispatch(Command::EditDraft("precious".into()));
    let first = intent_request(&s.dispatch(Command::Submit));
    let out = s.intent_persisted(ConversationId(1), &first, Err("disk full".into()));
    assert_eq!(backend_submits(&out), 0);
    let c = s.current().unwrap();
    assert_eq!(c.draft.text, "precious");
    assert_eq!(c.run, RunState::Idle);
    assert!(c.intent_error.as_ref().unwrap().contains("disk full"));
    assert!(s.availability().submit, "the user can try again");

    // A late duplicate acknowledgment for the failed request must do nothing.
    assert!(
        s.intent_persisted(ConversationId(1), &first, Ok(()))
            .effects
            .is_empty()
    );

    let second = intent_request(&s.dispatch(Command::Submit));
    assert_ne!(first, second, "each attempt has its own request id");
    assert!(s.current().unwrap().intent_error.is_none());
    s.dispatch(Command::DismissFailure);
}

#[test]
fn text_typed_after_pressing_send_is_not_lost() {
    let mut s = state();
    s.dispatch(Command::EditDraft("first".into()));
    let request = intent_request(&s.dispatch(Command::Submit));
    s.dispatch(Command::EditDraft("first and more".into()));
    let out = s.intent_persisted(ConversationId(1), &request, Ok(()));
    assert_eq!(backend_submits(&out), 1);
    assert_eq!(s.current().unwrap().draft.text, "first and more");
}

#[test]
fn acknowledgment_for_another_request_is_ignored() {
    let mut s = state();
    s.dispatch(Command::EditDraft("x".into()));
    s.dispatch(Command::Submit);
    let out = s.intent_persisted(ConversationId(1), &RequestId("other".into()), Ok(()));
    assert!(out.effects.is_empty() && out.notes.is_empty());
    assert!(s.current().unwrap().pending_intent.is_some());
}

#[test]
fn request_ids_are_unique_and_prefixed() {
    let mut s = state();
    s.set_request_prefix("abc".into());
    s.dispatch(Command::EditDraft("a".into()));
    let a = intent_request(&send_raw(&mut s));
    s.intent_persisted(ConversationId(1), &a, Err("x".into()));
    let b = intent_request(&send_raw(&mut s));
    assert!(a.0.starts_with("abc-") && b.0.starts_with("abc-"));
    assert_ne!(a, b);
}

fn send_raw(s: &mut AppState) -> Outcome {
    s.dispatch(Command::Submit)
}

#[test]
fn journal_follows_the_request_to_a_terminal_state_once() {
    let mut s = state();
    s.dispatch(Command::EditDraft("go".into()));
    send(&mut s, Command::Submit);
    let accepted = ev(&s, EventKind::Accepted);
    assert_eq!(
        journal_states(&s.apply_event(accepted)),
        vec![JournalState::Accepted]
    );
    let done = ev(&s, EventKind::Completed);
    assert_eq!(
        journal_states(&s.apply_event(done)),
        vec![JournalState::Completed]
    );
    // The request is settled; nothing can rewrite it.
    let again = BackendEvent {
        conversation: ConversationId(1),
        generation: s.current().unwrap().generation,
        op: None,
        kind: EventKind::Completed,
    };
    assert!(journal_states(&s.apply_event(again)).is_empty());
}

#[test]
fn lost_ack_and_rejection_are_journaled() {
    let mut s = state();
    s.dispatch(Command::EditDraft("go".into()));
    send(&mut s, Command::Submit);
    let lost = ev(&s, EventKind::AckLost);
    assert_eq!(
        journal_states(&s.apply_event(lost)),
        vec![JournalState::Unknown]
    );
    let resolved = ev(&s, EventKind::StatusResolved { accepted: false });
    assert_eq!(
        journal_states(&s.apply_event(resolved)),
        vec![JournalState::Rejected]
    );
}

#[test]
fn recovered_unresolved_request_is_unknown_and_never_resent() {
    let mut s = state();
    let r1 = RequestId("old-1".into());
    let out = s.restore_unresolved(ConversationId(1), r1.clone(), "half sent".into(), vec![]);
    assert_eq!(backend_submits(&out), 0);
    assert!(out.effects.is_empty());
    let c = s.current().unwrap();
    assert!(matches!(c.run, RunState::OutcomeUnknown { .. }));
    assert_eq!(c.last_submission.as_ref().unwrap().0, "half sent");
    let a = s.availability();
    assert!(a.check_status && !a.submit);

    // A newer unresolved request for the same conversation supersedes the older one.
    let out = s.restore_unresolved(
        ConversationId(1),
        RequestId("old-2".into()),
        "newer".into(),
        vec![],
    );
    assert!(out.effects.iter().any(|e| matches!(
        e,
        Effect::JournalState { request, state: JournalState::Superseded, .. } if *request == r1
    )));

    // Resolving as not accepted closes the newest request and offers an explicit retry.
    let resolved = ev(&s, EventKind::StatusResolved { accepted: false });
    assert_eq!(
        journal_states(&s.apply_event(resolved)),
        vec![JournalState::Rejected]
    );
    assert!(s.availability().retry);
}

#[test]
fn failed_queue_journal_write_puts_the_prompt_back() {
    let mut s = state();
    run_started(&mut s, "first");
    s.dispatch(Command::EditDraft("second".into()));
    s.dispatch(Command::QueueFollowUp);
    let done = ev(&s, EventKind::Completed);
    let out = s.apply_event(done);
    let request = intent_request(&out);
    assert!(s.current().unwrap().queue.is_empty());
    s.intent_persisted(ConversationId(1), &request, Err("busy".into()));
    let c = s.current().unwrap();
    assert_eq!(c.queue.len(), 1);
    assert_eq!(c.queue[0].text, "second");
}

#[test]
fn synced_replaces_items_only_once_opened() {
    let mut s = state();
    let item = |id: u64, text: &str| TranscriptItem {
        id: ItemId(id),
        at: 0,
        kind: ItemKind::Assistant {
            text: text.into(),
            streaming: false,
        },
    };
    let ev = |s: &AppState, items| {
        let c = s.current().unwrap();
        BackendEvent {
            conversation: c.id,
            generation: c.generation,
            op: None,
            kind: EventKind::Synced { items },
        }
    };
    // Not opened yet: ignored.
    let e = ev(&s, vec![item(1, "early")]);
    assert!(s.apply_event(e).notes.is_empty());
    assert!(s.current().unwrap().items.is_empty());

    let c = s.current().unwrap();
    let open = BackendEvent {
        conversation: c.id,
        generation: c.generation,
        op: None,
        kind: EventKind::Opened {
            items: vec![item(1, "one")],
            has_older: false,
            changes: vec![],
        },
    };
    s.apply_event(open);
    let e = ev(&s, vec![item(1, "one"), item(2, "two")]);
    let out = s.apply_event(e);
    assert_eq!(s.current().unwrap().items.len(), 2);
    assert!(out.notes.iter().any(|n| matches!(n, Note::ItemsReset(_))));

    // A stale generation never lands.
    let mut stale = ev(&s, vec![item(9, "stale")]);
    stale.generation += 1;
    s.apply_event(stale);
    assert_eq!(s.current().unwrap().items.len(), 2);
}

#[test]
fn output_preview_is_bounded_on_a_character_boundary() {
    let (short, cut) = preview_output("hi");
    assert_eq!((short.as_str(), cut), ("hi", false));
    let long = "\u{e9}".repeat(TOOL_OUTPUT_PREVIEW_BYTES); // two bytes per char
    let (preview, cut) = preview_output(&long);
    assert!(cut && preview.len() <= TOOL_OUTPUT_PREVIEW_BYTES);
    assert!(preview.chars().all(|c| c == '\u{e9}'));
}

#[test]
fn a_failed_open_is_visible_and_retryable() {
    let mut s = state();
    let c = s.current().unwrap();
    let (conv, generation) = (c.id, c.generation);
    let failed = BackendEvent {
        conversation: conv,
        generation,
        op: None,
        kind: EventKind::OpenFailed {
            message: "engine unreachable".into(),
        },
    };
    s.apply_event(failed);
    let c = s.current().unwrap();
    assert!(!c.opened);
    assert!(
        matches!(&c.items[0].kind, ItemKind::Notice { text, level: NoticeLevel::Error } if text == "engine unreachable")
    );
    // Selecting it again (after leaving) asks the backend to open it once more.
    s.dispatch(Command::SelectConversation(ConversationId(2)));
    let out = s.dispatch(Command::SelectConversation(conv));
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Backend(BackendRequest::Open { conversation, .. }) if *conversation == conv)));
    // Once open, a later failure does not clobber the real transcript.
    let c = s.current().unwrap();
    let open = BackendEvent {
        conversation: conv,
        generation: c.generation,
        op: None,
        kind: EventKind::Opened {
            items: vec![],
            has_older: false,
            changes: vec![],
        },
    };
    s.apply_event(open);
    let c = s.current().unwrap();
    let late = BackendEvent {
        conversation: conv,
        generation: c.generation,
        op: None,
        kind: EventKind::OpenFailed {
            message: "late".into(),
        },
    };
    s.apply_event(late);
    assert!(s.current().unwrap().items.is_empty());
}
