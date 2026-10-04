//! The GPUI entity that owns `AppState` and connects it to the controller's effect handler.

use desktop_core::*;
use gpui::{Context, EventEmitter};

pub type EffectHandler = Box<dyn FnMut(Effect, &mut Context<Model>)>;

pub struct Model {
    pub state: AppState,
    handler: Option<EffectHandler>,
}

impl EventEmitter<Note> for Model {}

impl Model {
    pub fn new(state: AppState) -> Self {
        Model {
            state,
            handler: None,
        }
    }

    /// Installed by the controller (desktop-app). Effects are executed outside render.
    pub fn set_effect_handler(&mut self, handler: EffectHandler) {
        self.handler = Some(handler);
    }

    pub fn dispatch(&mut self, command: Command, cx: &mut Context<Self>) {
        let outcome = self.state.dispatch(command);
        self.finish(outcome, cx);
    }

    pub fn apply_event(&mut self, event: BackendEvent, cx: &mut Context<Self>) {
        let outcome = self.state.apply_event(event);
        self.finish(outcome, cx);
    }

    pub fn draft_saved(
        &mut self,
        conversation: ConversationId,
        rev: u64,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        self.state.draft_saved(conversation, rev, result);
        cx.notify();
    }

    fn finish(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        let Outcome { effects, notes } = outcome;
        if let Some(mut handler) = self.handler.take() {
            for effect in effects {
                handler(effect, cx);
            }
            self.handler = Some(handler);
        }
        for note in notes {
            cx.emit(note);
        }
        cx.notify();
    }
}

/// Developer controls for the simulated backend. Installed as a GPUI global by the controller;
/// the command palette lists these under "Developer".
#[derive(Clone)]
pub struct DemoControls {
    /// (name, one-line description) of each scripted scenario.
    pub scenarios: Vec<(&'static str, &'static str)>,
    pub current_scenario: std::rc::Rc<dyn Fn() -> String>,
    pub set_scenario: std::rc::Rc<dyn Fn(&str)>,
    pub save_failure: std::rc::Rc<dyn Fn() -> bool>,
    pub set_save_failure: std::rc::Rc<dyn Fn(bool)>,
}

impl gpui::Global for DemoControls {}
