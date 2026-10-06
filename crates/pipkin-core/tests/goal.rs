use pipkin_core::*;

fn state() -> AppState {
    AppState::new(
        Bootstrap {
            projects: vec![Project {
                id: ProjectId(1),
                name: "demo".into(),
                path: "/demo".into(),
            }],
            models: vec![],
            conversations: vec![(ConversationId(1), ProjectId(1), "one".into(), 0)],
            now: 0,
        },
        Prefs::default(),
    )
}

fn settle(s: &mut AppState, out: Outcome) -> Outcome {
    let intents: Vec<_> = out
        .effects
        .iter()
        .filter_map(|effect| {
            if let Effect::JournalIntent {
                conversation,
                request,
                ..
            } = effect
            {
                Some((*conversation, request.clone()))
            } else {
                None
            }
        })
        .collect();
    let mut all = out;
    for (conversation, request) in intents {
        all.merge(s.intent_persisted(conversation, &request, Ok(())));
    }
    all
}

fn event(s: &AppState, kind: EventKind) -> BackendEvent {
    let c = s.current().unwrap();
    BackendEvent {
        conversation: c.id,
        generation: c.generation,
        op: c.run.op(),
        kind,
    }
}

#[test]
fn goal_continues_only_with_explicit_status_and_stops_when_met() {
    let mut s = state();
    let out = s.dispatch(Command::SetGoal("count to two".into()));
    let first = settle(&mut s, out);
    assert!(first.effects.iter().any(|e| matches!(e, Effect::Backend(BackendRequest::Submit { text, .. }) if text.contains("count to two"))));
    assert_eq!(s.current().unwrap().goal.as_ref().unwrap().turns, 1);
    s.apply_event(event(&s, EventKind::Accepted));
    s.apply_event(event(
        &s,
        EventKind::Token("One.\n[PIPKIN_GOAL_CONTINUE]".into()),
    ));
    let completed = event(&s, EventKind::Completed);
    let out = s.apply_event(completed);
    let next = settle(&mut s, out);
    assert!(
        next.effects
            .iter()
            .any(|e| matches!(e, Effect::Backend(BackendRequest::Submit { .. })))
    );
    s.apply_event(event(&s, EventKind::Accepted));
    s.apply_event(event(
        &s,
        EventKind::Token("Two.\n[PIPKIN_GOAL_MET]".into()),
    ));
    let done = s.apply_event(event(&s, EventKind::Completed));
    assert!(
        !done
            .effects
            .iter()
            .any(|e| matches!(e, Effect::JournalIntent { .. }))
    );
    assert_eq!(
        s.current()
            .unwrap()
            .goal
            .as_ref()
            .unwrap()
            .paused
            .as_deref(),
        Some("Goal met.")
    );
}

#[test]
fn clearing_while_goal_intent_is_uncommitted_never_sends_it() {
    let mut s = state();
    let out = s.dispatch(Command::SetGoal("pending".into()));
    let request = out
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::JournalIntent { request, .. } => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    s.dispatch(Command::ClearGoal);
    let acknowledged = s.intent_persisted(ConversationId(1), &request, Ok(()));
    assert!(acknowledged.effects.iter().any(|effect| matches!(
        effect,
        Effect::JournalState {
            state: JournalState::Cancelled,
            ..
        }
    )));
    assert!(
        !acknowledged
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Backend(BackendRequest::Submit { .. })))
    );
}

#[test]
fn restored_goal_is_visible_but_never_replays_without_an_explicit_update() {
    let mut s = state();
    s.restore_goal(ConversationId(1), "ship v1".into(), None);
    let opened = event(
        &s,
        EventKind::Opened {
            items: vec![],
            has_older: false,
            changes: vec![],
        },
    );
    let out = s.apply_event(opened);
    assert!(
        !out.effects
            .iter()
            .any(|effect| matches!(effect, Effect::JournalIntent { .. }))
    );
    assert!(s.current().unwrap().goal.as_ref().unwrap().paused.is_some());
    let out = s.dispatch(Command::SetGoal("ship v1".into()));
    assert!(
        out.effects
            .iter()
            .any(|effect| matches!(effect, Effect::JournalIntent { .. }))
    );
}

#[test]
fn missing_marker_pauses_and_clear_does_not_submit_again() {
    let mut s = state();
    let out = s.dispatch(Command::SetGoal("test".into()));
    settle(&mut s, out);
    s.apply_event(event(&s, EventKind::Accepted));
    s.apply_event(event(&s, EventKind::Token("ambiguous".into())));
    let completed = s.apply_event(event(&s, EventKind::Completed));
    assert!(
        !completed
            .effects
            .iter()
            .any(|e| matches!(e, Effect::JournalIntent { .. }))
    );
    assert!(s.current().unwrap().goal.as_ref().unwrap().paused.is_some());
    s.dispatch(Command::ClearGoal);
    assert!(s.current().unwrap().goal.is_none());
}
