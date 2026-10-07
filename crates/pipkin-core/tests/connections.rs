use pipkin_core::{onboarding::SignInProvider, *};

#[test]
fn credential_removal_requires_a_ready_real_engine() {
    let mut state = AppState::new(
        Bootstrap {
            projects: vec![],
            models: vec![],
            conversations: vec![],
            now: 0,
        },
        Prefs::default(),
    );
    state.mode = Mode::Demo;
    state.connection = Connection::Ready;
    let command = Command::RemoveSignIn(SignInProvider::Claude);
    assert!(state.dispatch(command.clone()).effects.is_empty());
    state.mode = Mode::Real;
    state.connection = Connection::Connecting;
    assert!(state.dispatch(command.clone()).effects.is_empty());
    state.connection = Connection::Ready;
    assert_eq!(
        state.dispatch(command).effects,
        vec![Effect::Backend(BackendRequest::RemoveSignIn(
            SignInProvider::Claude
        ))]
    );
}
