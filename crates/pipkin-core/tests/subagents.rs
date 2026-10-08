use pipkin_core::*;

fn state() -> AppState {
    state_in_mode(Mode::Real)
}

fn state_in_mode(mode: Mode) -> AppState {
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
    state.mode = mode;
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
fn advertised_demo_directory_opens_and_accepts_only_matching_synthetic_view() {
    for enabled in [true, false] {
        let mut s = state_in_mode(Mode::Demo);
        ready(&mut s);
        s.apply_event(event(
            &s,
            EventKind::SubagentsSynced {
                enabled,
                children: vec![child()],
            },
        ));
        s.dispatch(Command::EditDraft("parent draft".into()));
        s.read_only = Some("simulated read-only session".into());
        let parent = s.current().unwrap().clone();
        let availability = s.availability();
        let out = s.dispatch(Command::SelectSubagent(Some(10)));
        assert_eq!(
            out.effects,
            vec![Effect::Backend(BackendRequest::OpenSubagent {
                conversation: parent.id,
                generation: parent.generation,
                child: 10,
            })]
        );
        assert!(s.current().unwrap().subagents.loading);
        assert!(
            s.dispatch(Command::SelectSubagent(Some(999)))
                .effects
                .is_empty()
        );
        assert_eq!(s.current().unwrap().subagents.selected, Some(10));
        let items = vec![TranscriptItem {
            id: ItemId(101),
            at: 1,
            kind: ItemKind::Assistant {
                text: "Demo · simulated agent: synthetic child result".into(),
                streaming: false,
            },
        }];
        let matching = event(
            &s,
            EventKind::SubagentView {
                child: 10,
                items: items.clone(),
                live: false,
            },
        );
        let mut stale = matching.clone();
        stale.generation += 1;
        let mut wrong_parent = matching.clone();
        wrong_parent.conversation = ConversationId(2);
        let mut wrong_child = matching.clone();
        wrong_child.kind = EventKind::SubagentView {
            child: 11,
            items: items.clone(),
            live: false,
        };
        for rejected in [stale, wrong_parent, wrong_child] {
            assert!(s.apply_event(rejected).notes.is_empty());
            assert!(s.current().unwrap().subagents.loading);
            assert!(s.current().unwrap().subagents.items.is_empty());
        }
        s.apply_event(matching);
        let c = s.current().unwrap();
        assert_eq!(c.subagents.items, items);
        assert_eq!(c.subagents.viewed, Some(10));
        assert!(!c.subagents.loading);
        assert_eq!(c.subagents.previews.len(), 1);
        assert_eq!(c.draft, parent.draft);
        assert_eq!(c.run, parent.run);
        assert_eq!(c.items, parent.items);
        assert_eq!(s.mode, Mode::Demo);
        assert_eq!(s.availability(), availability);
        assert!(
            s.dispatch(Command::SetSubagentsEnabled(!enabled))
                .effects
                .is_empty()
        );
        assert_eq!(s.current().unwrap().subagents.enabled, Some(enabled));
        s.dispatch(Command::SelectSubagent(None));
        assert_eq!(s.current().unwrap().subagents.selected, None);
        assert!(s.current().unwrap().subagents.items.is_empty());
    }
}

#[test]
fn demo_sessions_without_an_open_advertised_service_reject_selection() {
    let mut s = state_in_mode(Mode::Demo);
    // An early directory event cannot advertise a service before the session opens.
    s.apply_event(event(
        &s,
        EventKind::SubagentsSynced {
            enabled: true,
            children: vec![child()],
        },
    ));
    assert_eq!(s.current().unwrap().subagents.enabled, None);
    for opened in [false, true] {
        s.conversations[0].opened = opened;
        // Even a directory row alone is not a capability advertisement.
        s.conversations[0].subagents.children = vec![child()];
        let before = s.current().unwrap().subagents.clone();
        for command in [
            Command::SelectSubagent(Some(10)),
            Command::SelectSubagent(None),
            Command::SetSubagentsEnabled(true),
        ] {
            let out = s.dispatch(command);
            assert!(out.effects.is_empty() && out.notes.is_empty());
            assert_eq!(s.current().unwrap().subagents, before);
        }
    }
    ready(&mut s);
    s.apply_event(event(
        &s,
        EventKind::SubagentsUnavailable("No simulated service".into()),
    ));
    let before = s.current().unwrap().subagents.clone();
    assert!(
        s.dispatch(Command::SelectSubagent(Some(10)))
            .effects
            .is_empty()
    );
    assert_eq!(s.current().unwrap().subagents, before);
    // Capability belongs to this conversation, not every demo session.
    ready(&mut s);
    s.dispatch(Command::SelectConversation(ConversationId(2)));
    assert!(
        s.dispatch(Command::SelectSubagent(Some(10)))
            .effects
            .is_empty()
    );
    assert_eq!(s.current().unwrap().subagents.selected, None);
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
