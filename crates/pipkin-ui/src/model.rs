//! The GPUI entity that owns `AppState` and connects it to the controller's effect handler.

use gpui::{Context, EventEmitter};
use pipkin_core::*;

pub type EffectHandler = Box<dyn FnMut(Effect, &mut Context<Model>)>;

pub struct Model {
    pub state: AppState,
    handler: Option<EffectHandler>,
    support_report: Option<String>,
}

impl EventEmitter<Note> for Model {}

impl Model {
    pub fn new(state: AppState) -> Self {
        Model {
            state,
            handler: None,
            support_report: None,
        }
    }

    /// Installed by the controller (pipkin-app). Effects are executed outside render.
    pub fn set_effect_handler(&mut self, handler: EffectHandler) {
        self.handler = Some(handler);
    }

    /// Prepared by the controller outside render; never composed from conversation/error text.
    pub fn set_support_report(&mut self, report: String) {
        self.support_report = Some(report);
    }

    pub fn support_report(&self) -> Option<&str> {
        self.support_report.as_deref()
    }

    pub fn dispatch(&mut self, command: Command, cx: &mut Context<Self>) {
        let outcome = self.state.dispatch(command);
        self.finish(outcome, cx);
    }

    pub fn apply_event(&mut self, event: BackendEvent, cx: &mut Context<Self>) {
        let outcome = self.state.apply_event(event);
        self.finish(outcome, cx);
    }

    /// Run a state change that produces an `Outcome` (effects and notes) outside a command.
    pub fn mutate(&mut self, f: impl FnOnce(&mut AppState) -> Outcome, cx: &mut Context<Self>) {
        let outcome = f(&mut self.state);
        self.finish(outcome, cx);
    }

    pub fn set_connection(&mut self, connection: Connection, cx: &mut Context<Self>) {
        let outcome = self.state.set_connection(connection);
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
