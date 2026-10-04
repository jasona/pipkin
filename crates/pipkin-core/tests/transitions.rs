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

fn run_started(s: &mut AppState, text: &str) -> OperationId {
    s.dispatch(Command::EditDraft(text.into()));
    s.dispatch(Command::Submit);
    let op = s.current().unwrap().run.op().unwrap();
    let e = ev(s, EventKind::Accepted);
    s.apply_event(e);
    op
}

#[test]
fn submit_moves_draft_into_transcript_and_runs() {
    let mut s = state();
    s.dispatch(Command::EditDraft("fix the test".into()));
    let out = s.dispatch(Command::Submit);
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
    s.dispatch(Command::Submit);
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
    s.dispatch(Command::Submit);
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
    let out = s.dispatch(Command::Retry);
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
    let out = s.apply_event(e);
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
