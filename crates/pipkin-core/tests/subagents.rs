use pipkin_core::*;

fn state() -> AppState {
    let mut state = AppState::new(
        Bootstrap {
            projects: vec![Project {
                id: ProjectId(1),
                name: "P".into(),
                path: "/tmp/p".into(),
            }],
            conversations: vec![
                (ConversationId(1), ProjectId(1), "one".into(), 1),
                (ConversationId(2), ProjectId(1), "two".into(), 2),
            ],
            models: vec![],
            now: 1,
        },
        Prefs::default(),
    );
    state.mode = Mode::Real;
    state.set_connection(Connection::Ready);
    state.dispatch(Command::SelectConversation(ConversationId(1)));
    state
}

fn event(s: &AppState, kind: EventKind) -> BackendEvent {
    let c = s.current().unwrap();
    BackendEvent {
        conversation: c.id,
        generation: c.generation,
        op: None,
        kind,
    }
}

fn child() -> SubagentInfo {
    SubagentInfo {
        id: 10,
        task_id: 7,
        call_id: "call-7".into(),
        task: "Investigate".into(),
        status: "running".into(),
    }
}

fn ready(s: &mut AppState) {
    s.apply_event(event(
        s,
        EventKind::Opened {
            items: vec![],
            has_older: false,
            changes: vec![],
        },
    ));
    s.apply_event(event(
        s,
        EventKind::SubagentsSynced {
            enabled: true,
            children: vec![child()],
        },
    ));
}

#[test]
fn unsupported_session_cannot_toggle_or_open_and_disable_is_engine_authoritative() {
    let mut s = state();
    assert!(
        s.dispatch(Command::SetSubagentsEnabled(false))
            .effects
            .is_empty()
    );
    assert!(
        s.dispatch(Command::SelectSubagent(Some(10)))
            .effects
            .is_empty()
    );
    ready(&mut s);
    let effects = s.dispatch(Command::SetSubagentsEnabled(false)).effects;
    assert!(matches!(
        effects.as_slice(),
        [Effect::Backend(BackendRequest::SetSubagentsEnabled {
            conversation: ConversationId(1),
            enabled: false,
            ..
        })]
    ));
    assert_eq!(s.current().unwrap().subagents.enabled, Some(true));
    s.apply_event(event(
        &s,
        EventKind::SubagentsSynced {
            enabled: false,
            children: vec![child()],
        },
    ));
    assert_eq!(s.current().unwrap().subagents.enabled, Some(false));
    // Existing child remains visible even though new engine calls have been disabled.
    assert!(matches!(
        s.dispatch(Command::SelectSubagent(Some(10)))
            .effects
            .as_slice(),
        [Effect::Backend(BackendRequest::OpenSubagent {
            child: 10,
            ..
        })]
    ));
    s.apply_event(event(
        &s,
        EventKind::SubagentView {
            child: 10,
            items: vec![TranscriptItem {
                id: ItemId(1),
                at: 0,
                kind: ItemKind::Assistant {
                    text: "x".repeat(5000),
                    streaming: true,
                },
            }],
            live: true,
        },
    ));
    assert_eq!(s.current().unwrap().subagents.previews[0].text.len(), 4096);
    assert!(s.current().unwrap().subagents.previews[0].clipped);
    assert_eq!(s.current().unwrap().subagents.enabled, Some(false));
    assert!(s.current().unwrap().subagents.live);
}

#[test]
fn child_identity_generation_and_parent_are_guarded_and_draft_is_unchanged() {
    let mut s = state();
    ready(&mut s);
    s.dispatch(Command::EditDraft("parent draft".into()));
    let old = event(
        &s,
        EventKind::SubagentView {
            child: 10,
            items: vec![],
            live: true,
        },
    );
    s.dispatch(Command::SelectSubagent(Some(10)));
    s.apply_event(event(
        &s,
        EventKind::SubagentView {
            child: 11,
            items: vec![],
            live: true,
        },
    ));
    assert!(s.current().unwrap().subagents.loading);
    s.apply_event(event(
        &s,
        EventKind::SubagentView {
            child: 10,
            items: vec![],
            live: true,
        },
    ));
    assert!(s.current().unwrap().subagents.live);
    s.dispatch(Command::SelectConversation(ConversationId(2)));
    s.apply_event(old);
    assert!(
        s.conversation(ConversationId(2))
            .unwrap()
            .subagents
            .items
            .is_empty()
    );
    s.dispatch(Command::SelectConversation(ConversationId(1)));
    assert_eq!(s.current().unwrap().draft.text, "parent draft");
    // The previous attachment may not overwrite the newly attached child.
    let mut stale = event(
        &s,
        EventKind::SubagentViewFailed {
            child: 10,
            reason: "old".into(),
        },
    );
    stale.generation -= 1;
    s.apply_event(stale);
    assert!(s.current().unwrap().subagents.error.is_none());
    assert!(
        s.dispatch(Command::SelectSubagent(Some(999)))
            .effects
            .is_empty()
    );
    assert_eq!(s.current().unwrap().subagents.selected, Some(10));
}
