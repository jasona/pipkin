use pipkin_core::*;

#[test]
fn session_usage_is_conversation_scoped_and_stale_updates_are_dropped() {
    let mut state = AppState::new(
        Bootstrap {
            projects: vec![Project {
                id: ProjectId(1),
                name: "one".into(),
                path: "/one".into(),
            }],
            models: vec![],
            conversations: vec![
                (ConversationId(1), ProjectId(1), "first".into(), 1),
                (ConversationId(2), ProjectId(1), "second".into(), 2),
            ],
            now: 3,
        },
        Prefs::default(),
    );
    let generation = state.conversation(ConversationId(1)).unwrap().generation;
    let usage = SessionUsage {
        session_id: "engine-1".into(),
        models: vec![(
            "p/m".into(),
            UsageAmount {
                input: 5,
                output: 1,
                cache_read: 0,
                cache_write: 0,
                total_tokens: 6,
                cost_usd: Some(0.03),
            },
        )],
        tools: vec![],
    };
    let event = |generation, kind| BackendEvent {
        conversation: ConversationId(1),
        generation,
        op: None,
        kind,
    };
    // Usage cannot arrive before the attached transcript, and must not leak to conversation 2.
    state.apply_event(event(generation, EventKind::UsageSynced(usage.clone())));
    assert!(
        state
            .conversation(ConversationId(1))
            .unwrap()
            .usage
            .is_none()
    );
    state.apply_event(event(
        generation,
        EventKind::Opened {
            items: vec![],
            has_older: false,
            changes: vec![],
        },
    ));
    state.apply_event(event(generation, EventKind::UsageSynced(usage.clone())));
    state.dispatch(Command::SelectConversation(ConversationId(2)));
    assert_eq!(
        state
            .conversation(ConversationId(1))
            .unwrap()
            .usage
            .as_ref(),
        Some(&usage)
    );
    assert!(
        state
            .conversation(ConversationId(2))
            .unwrap()
            .usage
            .is_none()
    );
    let newer = SessionUsage {
        session_id: "wrong".into(),
        ..usage
    };
    state.apply_event(event(generation + 1, EventKind::UsageSynced(newer)));
    assert_eq!(
        state
            .conversation(ConversationId(1))
            .unwrap()
            .usage
            .as_ref()
            .unwrap()
            .session_id,
        "engine-1"
    );
}
