use std::time::Duration;

use gpui::{
    App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, MouseButton, MouseUpEvent, ParentElement, PathPromptOptions,
    Render, ScrollHandle, Styled, Subscription, Task, UniformListScrollHandle, Window, deferred,
    div, prelude::*, px,
};
use pipkin_core::*;

use super::actions::*;
use super::commands::Run;
use crate::model::{DemoControls, Model};
use crate::text::{ComposerEditor, ComposerEvent};
use crate::theme::{ActiveTheme, Theme};
use crate::transcript::TranscriptView;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    None,
    Palette,
    Model,
    Project,
    Rename(ConversationId),
    Prefs,
    About,
    /// A question an extension asked, for the first one waiting.
    Question,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Nav,
    Inspector,
}

#[derive(Clone, Copy)]
pub enum Split {
    Nav,
    Inspector,
}

struct DragGhost;
impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub const MIN_CENTER: f32 = 480.0;
pub const DOCK_NAV_MIN_WIDTH: f32 = 900.0;
const DIVIDER_WIDTH: f32 = 5.0;

// The inspector can use all available space except the navigation pane and a readable centre.
fn inspector_drag_width(x: f32, total: f32, nav: f32) -> f32 {
    let available = total - nav - 2.0 * DIVIDER_WIDTH - MIN_CENTER;
    (total - x).clamp(280.0, available.max(280.0))
}

#[cfg(test)]
mod resize_tests {
    use super::inspector_drag_width;

    #[test]
    fn inspector_uses_available_room_instead_of_a_fixed_ceiling() {
        assert_eq!(inspector_drag_width(500.0, 1920.0, 240.0), 1190.0);
        assert_eq!(inspector_drag_width(1800.0, 1920.0, 240.0), 280.0);
        assert_eq!(inspector_drag_width(1100.0, 1200.0, 240.0), 280.0);
        assert_eq!(inspector_drag_width(0.0, 1200.0, 240.0), 470.0);
    }
}

pub struct Workspace {
    pub(super) model: Entity<Model>,
    pub(super) transcript: Entity<TranscriptView>,
    pub(super) composer: Entity<ComposerEditor>,
    pub(super) nav_search: Entity<ComposerEditor>,
    pub(super) overlay_input: Entity<ComposerEditor>,
    pub(super) overlay: Overlay,
    pub(super) overlay_sel: usize,
    /// Questions the person put aside; they are not offered again on their own.
    pub(super) question_dismissed: Vec<String>,
    pub(super) restore_focus: Option<FocusHandle>,
    pub(super) temp_panel: Option<Panel>,
    pub(super) panel_restore: Option<FocusHandle>,
    pub(super) root_focus: FocusHandle,
    pub(super) menu_focus: FocusHandle,
    pub(super) panel_focus: FocusHandle,
    synced: (Option<ConversationId>, u64),
    flush_task: Option<Task<()>>,
    /// Saves the window size once resizing has settled.
    size_task: Option<Task<()>>,
    /// The conversation search box is showing (it also shows while it holds a query).
    pub(super) search_open: bool,
    pub(super) toast: Option<String>,
    toast_task: Option<Task<()>>,
    pub(super) diff_scroll: UniformListScrollHandle,
    pub(super) nav_scroll: ScrollHandle,
    /// The model/project menu's list, so the highlighted row is kept in view.
    pub(super) menu_scroll: ScrollHandle,
    /// Where the model button was last painted, so its menu opens right above it.
    pub(super) model_button: Option<gpui::Bounds<gpui::Pixels>>,
    pub(super) changes_scroll: ScrollHandle,
    pub(super) diff_rows: DiffRows,
    live_nav: Option<f32>,
    live_insp: Option<f32>,
    last_change_sel: Option<(ConversationId, Option<usize>)>,
    _subs: Vec<Subscription>,
}

#[derive(Default)]
pub struct DiffRows {
    pub key: Option<(ConversationId, usize, u32, u32)>,
    pub rows: Vec<(u32, u32)>,
    pub widest: usize,
    pub change: Option<std::rc::Rc<FileChange>>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root_focus.clone()
    }
}

impl Workspace {
    pub fn new(model: Entity<Model>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let transcript = cx.new(|cx| TranscriptView::new(model.clone(), window, cx));
        let composer = cx.new(|cx| ComposerEditor::new(window, cx));
        let nav_search = cx.new(|cx| ComposerEditor::single_line(window, cx));
        let overlay_input = cx.new(|cx| ComposerEditor::single_line(window, cx));
        nav_search.update(cx, |e, cx| {
            e.set_placeholder("Search conversations");
            e.set_label("Search conversations", cx);
        });
        overlay_input.update(cx, |e, cx| {
            e.set_label("Command palette or dialog input", cx)
        });
        composer.update(cx, |e, _| e.set_placeholder("Ask anything\u{2026}"));
        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(&composer, window, Self::on_composer_event));
        subs.push(cx.subscribe_in(&nav_search, window, Self::on_search_event));
        subs.push(cx.subscribe_in(&overlay_input, window, Self::on_overlay_input_event));
        subs.push(
            cx.observe_window_bounds(window, |this, window, cx| this.on_window_bounds(window, cx)),
        );
        subs.push(cx.observe_in(&model, window, |this, _, window, cx| {
            this.on_model(window, cx)
        }));
        let mut ws = Workspace {
            model,
            transcript,
            composer,
            nav_search,
            overlay_input,
            overlay: Overlay::None,
            overlay_sel: 0,
            question_dismissed: Vec::new(),
            restore_focus: None,
            temp_panel: None,
            panel_restore: None,
            root_focus: cx.focus_handle(),
            menu_focus: cx.focus_handle(),
            panel_focus: cx.focus_handle(),
            synced: (None, u64::MAX),
            flush_task: None,
            size_task: None,
            search_open: false,
            toast: None,
            toast_task: None,
            diff_scroll: UniformListScrollHandle::new(),
            nav_scroll: ScrollHandle::new(),
            menu_scroll: ScrollHandle::new(),
            model_button: None,
            changes_scroll: ScrollHandle::new(),
            diff_rows: DiffRows::default(),
            live_nav: None,
            live_insp: None,
            last_change_sel: None,
            _subs: subs,
        };
        ws.on_model(window, cx);
        let composer_focus = ws.composer.read(cx).focus_handle(cx);
        window.focus(&composer_focus, cx);
        ws
    }

    pub(super) fn state<'a>(&self, cx: &'a App) -> &'a AppState {
        &self.model.read(cx).state
    }

    pub(super) fn dispatch(&self, command: Command, cx: &mut App) {
        // Opening a hit ends the search, so the box that held it is emptied too.
        let ends_search = matches!(command, Command::OpenSearchHit(_));
        self.model.update(cx, |m, cx| m.dispatch(command, cx));
        if ends_search {
            self.nav_search.update(cx, |e, cx| e.set_text("", cx));
        }
    }

    pub(super) fn focus_composer(&self, window: &mut Window, cx: &mut App) {
        let h = self.composer.read(cx).focus_handle(cx);
        window.focus(&h, cx);
    }

    pub(super) fn show_toast(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.toast = Some(text.into());
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    // ------------------------------------------------------------ model → view

    fn on_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (prefs, current, title, run, change_sel) = {
            let s = self.state(cx);
            let c = s.current();
            (
                s.prefs.clone(),
                c.map(|c| (c.id, c.draft.sync_epoch, c.draft.text.clone())),
                c.map(|c| c.title.clone()),
                c.map(|c| c.run.clone()),
                c.map(|c| (c.id, c.selected_change)),
            )
        };
        let wanted = Theme::new(prefs.theme, prefs.text_size, prefs.reduced_motion);
        let differs = cx
            .try_global::<Theme>()
            .map(|t| {
                t.choice != wanted.choice
                    || t.scale != wanted.scale
                    || t.reduced_motion != wanted.reduced_motion
            })
            .unwrap_or(true);
        if differs {
            cx.set_global(wanted);
            cx.refresh_windows();
        }
        match current {
            Some((id, epoch, text)) => {
                if self.synced != (Some(id), epoch) {
                    self.synced = (Some(id), epoch);
                    self.composer.update(cx, |e, cx| e.set_text(&text, cx));
                }
            }
            None => {
                if self.synced.0.is_some() || self.synced.1 == u64::MAX {
                    self.synced = (None, 0);
                    self.composer.update(cx, |e, cx| e.set_text("", cx));
                }
            }
        }
        let locked = matches!(
            run,
            Some(RunState::Submitting { .. } | RunState::OutcomeUnknown { .. }) | None
        );
        self.composer.update(cx, |e, cx| e.set_disabled(locked, cx));
        window.set_window_title(&match title {
            Some(t) => format!("{t} — Pipkin"),
            None => "Pipkin".to_string(),
        });
        // A change selected from the transcript must be visible even when the inspector is not
        // docked; this opens the temporary panel without touching transcript scroll or draft.
        if change_sel != self.last_change_sel {
            let was = self.last_change_sel;
            self.last_change_sel = change_sel;
            if let (Some((id, Some(_))), Some((pid, _))) = (change_sel, was)
                && id == pid
                && !self.can_dock_inspector(window)
                && self.temp_panel != Some(Panel::Inspector)
            {
                self.open_panel(Panel::Inspector, window, cx);
            }
        }
        cx.notify();
    }

    fn on_composer_event(
        &mut self,
        _: &Entity<ComposerEditor>,
        ev: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            ComposerEvent::Changed => {
                let text = self.composer.read(cx).text();
                self.dispatch(Command::EditDraft(text), cx);
                self.schedule_flush(cx);
            }
            ComposerEvent::Submit => self.submit_primary(window, cx),
            ComposerEvent::Escape => {}
            _ => {}
        }
    }

    /// Enter: send when idle, steer when a run is active.
    pub(super) fn submit_primary(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let a = self.state(cx).availability();
        if a.submit {
            self.dispatch(Command::Submit, cx);
        } else if a.steer {
            self.dispatch(Command::Steer, cx);
        }
    }

    /// Remember the window's size, once resizing has paused. The viewport is the real size:
    /// on Wayland `window_bounds()` calls an ordinary resize "maximized" and keeps the old
    /// size, so it cannot be trusted here. Fullscreen is the screen's size, not the person's
    /// choice, so it is skipped.
    fn on_window_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_fullscreen() {
            return;
        }
        let viewport = window.viewport_size();
        let (w, h) = (f32::from(viewport.width), f32::from(viewport.height));
        let model = self.model.clone();
        self.size_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(600))
                .await;
            model.update(cx, |m, cx| m.dispatch(Command::SetWindowSize(w, h), cx));
        }));
    }

    fn schedule_flush(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.state(cx).selected else {
            return;
        };
        let model = self.model.clone();
        self.flush_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            model.update(cx, |m, cx| m.dispatch(Command::FlushDraft(id), cx));
        }));
    }

    fn on_search_event(
        &mut self,
        _: &Entity<ComposerEditor>,
        ev: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            ComposerEvent::Changed => {
                let q = self.nav_search.read(cx).text();
                self.dispatch(Command::SetSearch(q), cx);
            }
            ComposerEvent::Escape => {
                self.search_open = false;
                self.nav_search.update(cx, |e, cx| e.set_text("", cx));
                self.dispatch(Command::SetSearch(String::new()), cx);
                self.focus_composer(window, cx);
            }
            ComposerEvent::Submit | ComposerEvent::Down => {
                let first = self.state(cx).visible_conversations().first().map(|c| c.id);
                if let Some(id) = first {
                    self.dispatch(Command::SelectConversation(id), cx);
                    self.focus_composer(window, cx);
                }
            }
            _ => {}
        }
    }

    fn on_overlay_input_event(
        &mut self,
        _: &Entity<ComposerEditor>,
        ev: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            ComposerEvent::Changed => {
                self.overlay_sel = 0;
                cx.notify();
            }
            ComposerEvent::Up => self.move_selection(-1, cx),
            ComposerEvent::Down => self.move_selection(1, cx),
            ComposerEvent::Escape => self.close_overlay(window, cx),
            ComposerEvent::Submit => match self.overlay {
                Overlay::Rename(id) => {
                    let title = self.overlay_input.read(cx).text();
                    self.dispatch(Command::RenameConversation(id, title), cx);
                    self.close_overlay(window, cx);
                }
                Overlay::Question => self.confirm_selection(window, cx),
                _ => self.confirm_selection(window, cx),
            },
        }
    }

    // ------------------------------------------------------------- layout rules

    pub(super) fn nav_width(&self, cx: &App) -> f32 {
        self.live_nav.unwrap_or(self.state(cx).prefs.nav_width)
    }

    pub(super) fn inspector_width(&self, cx: &App) -> f32 {
        self.live_insp
            .unwrap_or(self.state(cx).prefs.inspector_width)
    }

    pub(super) fn nav_docked(&self, window: &Window) -> bool {
        f32::from(window.viewport_size().width) >= DOCK_NAV_MIN_WIDTH
    }

    fn can_dock_inspector(&self, window: &Window) -> bool {
        let w = f32::from(window.viewport_size().width);
        self.nav_docked(window) && w - self.nav_width_hint() - 400.0 >= MIN_CENTER
    }

    fn nav_width_hint(&self) -> f32 {
        self.live_nav.unwrap_or(240.0)
    }

    pub(super) fn inspector_docked(&self, window: &Window, cx: &App) -> bool {
        let w = f32::from(window.viewport_size().width);
        let s = self.state(cx);
        s.prefs.inspector_open
            && self.nav_docked(window)
            && w - s.prefs.nav_width.min(420.0) - self.inspector_width(cx) >= MIN_CENTER
    }

    // ----------------------------------------------------------------- overlays

    pub(super) fn open_overlay(&mut self, o: Overlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Overlay::None {
            self.restore_focus = window.focused(cx);
        }
        self.overlay = o;
        self.overlay_sel = 0;
        match o {
            Overlay::Palette => {
                self.overlay_input.update(cx, |e, cx| {
                    e.set_text("", cx);
                    e.set_placeholder("Type a command…");
                });
                let h = self.overlay_input.read(cx).focus_handle(cx);
                window.focus(&h, cx);
            }
            Overlay::Rename(id) => {
                let title = self
                    .state(cx)
                    .conversation(id)
                    .map(|c| c.title.clone())
                    .unwrap_or_default();
                self.overlay_input.update(cx, |e, cx| {
                    e.set_text(&title, cx);
                    e.set_placeholder("Conversation title");
                    e.select_all(cx);
                });
                let h = self.overlay_input.read(cx).focus_handle(cx);
                window.focus(&h, cx);
            }
            Overlay::Model => {
                let cur = self.state(cx).prefs.model.clone();
                self.overlay_sel = self
                    .state(cx)
                    .models
                    .iter()
                    .position(|m| Some(&m.id) == cur.as_ref())
                    .unwrap_or(0);
                self.menu_scroll.scroll_to_item(self.overlay_sel);
                window.focus(&self.menu_focus, cx);
            }
            Overlay::Project | Overlay::Prefs | Overlay::About => {
                window.focus(&self.menu_focus, cx)
            }
            Overlay::Question => {
                let question = self.waiting_question(cx, true);
                match question {
                    Some(q) if q.kind == UiRequestKind::Input => {
                        self.overlay_input.update(cx, |e, cx| {
                            e.set_text(q.default_value.as_deref().unwrap_or(""), cx);
                            e.set_placeholder(q.placeholder.as_deref().unwrap_or("Your answer"));
                            e.select_all(cx);
                        });
                        let h = self.overlay_input.read(cx).focus_handle(cx);
                        window.focus(&h, cx);
                    }
                    _ => window.focus(&self.menu_focus, cx),
                }
            }
            Overlay::None => {}
        }
        cx.notify();
    }

    /// The first question an extension is waiting on in the open conversation. Questions put
    /// aside are skipped unless `including_dismissed`.
    pub(super) fn waiting_question(
        &self,
        cx: &App,
        including_dismissed: bool,
    ) -> Option<UiRequest> {
        self.state(cx).current().and_then(|c| {
            c.ui_requests
                .iter()
                .find(|q| including_dismissed || !self.question_dismissed.contains(&q.id))
                .cloned()
        })
    }

    /// Offer a waiting question without being asked, once.
    fn offer_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay != Overlay::None || self.waiting_question(cx, false).is_none() {
            return;
        }
        let this = cx.entity();
        window.defer(cx, move |window, cx| {
            this.update(cx, |t, cx| {
                if t.overlay == Overlay::None && t.waiting_question(cx, false).is_some() {
                    t.open_overlay(Overlay::Question, window, cx);
                }
            })
        });
    }

    pub(super) fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Overlay::None {
            return;
        }
        if self.overlay == Overlay::Question
            && let Some(q) = self.waiting_question(cx, true)
            && !self.question_dismissed.contains(&q.id)
        {
            self.question_dismissed.push(q.id);
        }
        if self.overlay == Overlay::About {
            self.overlay = Overlay::Prefs;
            window.focus(&self.menu_focus, cx);
            cx.notify();
            return;
        }
        self.overlay = Overlay::None;
        match self.restore_focus.take() {
            Some(h) => window.focus(&h, cx),
            None => self.focus_composer(window, cx),
        }
        cx.notify();
    }

    pub(super) fn open_panel(&mut self, p: Panel, window: &mut Window, cx: &mut Context<Self>) {
        if self.temp_panel.is_none() {
            self.panel_restore = window.focused(cx);
        }
        self.temp_panel = Some(p);
        window.focus(&self.panel_focus, cx);
        cx.notify();
    }

    pub(super) fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.temp_panel.take().is_some() {
            match self.panel_restore.take() {
                Some(h) => window.focus(&h, cx),
                None => self.focus_composer(window, cx),
            }
            cx.notify();
        }
    }

    pub(super) fn move_selection(&mut self, delta: i32, cx: &mut Context<Self>) {
        let n = self.overlay_len(cx).max(1) as i32;
        self.overlay_sel = (self.overlay_sel as i32 + delta).rem_euclid(n) as usize;
        self.menu_scroll.scroll_to_item(self.overlay_sel);
        cx.notify();
    }

    // ------------------------------------------------------------------ actions

    pub(super) fn run_command(&mut self, run: &Run, window: &mut Window, cx: &mut Context<Self>) {
        match run {
            Run::Action(a) => {
                let a = a.boxed_clone();
                window.dispatch_action(a, cx);
            }
            Run::Dispatch(c) => self.dispatch(c.clone(), cx),
            Run::DemoScenario(name) => {
                if let Some(d) = cx.try_global::<DemoControls>().cloned() {
                    (d.set_scenario)(name);
                    self.show_toast(format!("Demo scenario: {name}"), cx);
                }
            }
            Run::ToggleSaveFailure => {
                if let Some(d) = cx.try_global::<DemoControls>().cloned() {
                    let on = !(d.save_failure)();
                    (d.set_save_failure)(on);
                    self.show_toast(
                        if on {
                            "Draft save failures injected"
                        } else {
                            "Draft saves restored"
                        },
                        cx,
                    );
                }
            }
        }
    }

    fn on_open_palette(&mut self, _: &OpenPalette, window: &mut Window, cx: &mut Context<Self>) {
        self.open_overlay(Overlay::Palette, window, cx);
    }
    fn on_new_conversation(
        &mut self,
        _: &NewConversation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_panel(window, cx);
        self.dispatch(Command::NewConversation, cx);
        self.focus_composer(window, cx);
    }
    fn on_toggle_nav(&mut self, _: &ToggleNav, window: &mut Window, cx: &mut Context<Self>) {
        if self.nav_docked(window) {
            let h = self.nav_search.read(cx).focus_handle(cx);
            window.focus(&h, cx);
        } else if self.temp_panel == Some(Panel::Nav) {
            self.close_panel(window, cx);
        } else {
            self.open_panel(Panel::Nav, window, cx);
        }
    }
    fn on_toggle_inspector(
        &mut self,
        _: &ToggleInspector,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.can_dock_inspector(window) {
            let open = !self.state(cx).prefs.inspector_open;
            self.dispatch(Command::SetInspectorOpen(open), cx);
        } else if self.temp_panel == Some(Panel::Inspector) {
            self.close_panel(window, cx);
        } else {
            self.open_panel(Panel::Inspector, window, cx);
        }
    }
    fn on_focus_composer(
        &mut self,
        _: &FocusComposer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_composer(window, cx);
    }
    fn on_focus_transcript(
        &mut self,
        _: &FocusTranscript,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let h = self.transcript.read(cx).focus_handle(cx);
        window.focus(&h, cx);
    }
    fn on_prefs(&mut self, _: &OpenPreferences, window: &mut Window, cx: &mut Context<Self>) {
        self.open_overlay(Overlay::Prefs, window, cx);
    }
    fn on_rename(&mut self, _: &RenameConversation, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.state(cx).selected {
            self.open_overlay(Overlay::Rename(id), window, cx);
        }
    }
    fn on_cancel(&mut self, _: &CancelRun, _: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Command::Cancel, cx);
    }
    fn on_queue(&mut self, _: &QueueFollowUp, _: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Command::QueueFollowUp, cx);
    }
    fn on_steer(&mut self, _: &SteerRun, _: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Command::Steer, cx);
    }
    fn on_jump(&mut self, _: &JumpToLatest, window: &mut Window, cx: &mut Context<Self>) {
        self.transcript
            .update(cx, |t, cx| t.jump_to_latest(window, cx));
    }
    fn on_model_menu(&mut self, _: &OpenModelMenu, window: &mut Window, cx: &mut Context<Self>) {
        self.open_overlay(Overlay::Model, window, cx);
    }
    fn on_answer_question(
        &mut self,
        _: &AnswerQuestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.waiting_question(cx, true).is_some() {
            self.question_dismissed.clear();
            self.open_overlay(Overlay::Question, window, cx);
        }
    }
    fn on_open_project(&mut self, _: &OpenProject, _: &mut Window, cx: &mut Context<Self>) {
        self.open_project(cx);
    }

    /// Choose a project folder and add it. The engine resolves a session's directory through
    /// symlinks, so the folder is resolved the same way here, which keeps one folder one project.
    pub(super) fn open_project(&mut self, cx: &mut Context<Self>) {
        if !self.state(cx).can_create {
            return;
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open project folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let paths = match rx.await {
                Ok(Ok(Some(p))) => p,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(e)) => {
                    this.update(cx, |this, cx| {
                        this.show_toast(format!("Folder picker unavailable: {e}"), cx)
                    })
                    .ok();
                    return;
                }
            };
            let Some(first) = paths.into_iter().next() else {
                return;
            };
            let resolved = cx
                .background_spawn(async move { std::fs::canonicalize(&first).map(|p| (first, p)) })
                .await;
            this.update(cx, |this, cx| match resolved {
                Ok((_, real)) if real.is_dir() => {
                    this.dispatch(Command::AddProject(real.display().to_string()), cx)
                }
                Ok((chosen, _)) => {
                    this.show_toast(format!("{} is not a folder", chosen.display()), cx)
                }
                Err(e) => this.show_toast(format!("Cannot open that folder: {e}"), cx),
            })
            .ok();
        })
        .detach();
    }

    fn on_attach(&mut self, _: &AttachFiles, _: &mut Window, cx: &mut Context<Self>) {
        self.attach_files(cx);
    }
    fn on_next(&mut self, _: &NextConversation, _: &mut Window, cx: &mut Context<Self>) {
        self.step_conversation(1, cx);
    }
    fn on_prev(&mut self, _: &PrevConversation, _: &mut Window, cx: &mut Context<Self>) {
        self.step_conversation(-1, cx);
    }
    fn on_close_overlay(&mut self, _: &CloseOverlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay != Overlay::None {
            self.close_overlay(window, cx);
        } else {
            self.close_panel(window, cx);
        }
    }
    fn on_menu_up(&mut self, _: &MenuUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }
    fn on_menu_down(&mut self, _: &MenuDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }
    fn on_menu_confirm(&mut self, _: &MenuConfirm, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_selection(window, cx);
    }
    fn on_log_stats(&mut self, _: &LogStats, _: &mut Window, cx: &mut Context<Self>) {
        let s = self.transcript.read(cx).stats(cx);
        let rss_kb = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|t| {
                t.lines()
                    .find(|l| l.starts_with("VmRSS"))
                    .and_then(|l| l.split_whitespace().nth(1).map(str::to_string))
            })
            .unwrap_or_default();
        if let Some(l) = crate::text::latency::stats() {
            log::warn!(
                "perf: input-to-frame (handler call to composer paint end; CPU, not display presentation) n={} p50={:?} p95={:?} max={:?}",
                l.count,
                l.p50,
                l.p95,
                l.max
            );
        }
        let msg = format!(
            "perf: transcript frames={} p50={:?} p95={:?} (CPU prepaint+paint, not display presentation) mounted_rows={} cached_blocks={} following={} rss_kb={}",
            s.frames,
            s.frame_p50,
            s.frame_p95,
            s.mounted_rows,
            s.cached_blocks,
            s.following,
            rss_kb
        );
        log::warn!("{msg}");
        self.show_toast("Performance stats written to the log", cx);
    }
    fn on_quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    fn step_conversation(&mut self, delta: i32, cx: &mut Context<Self>) {
        let (ids, cur) = {
            let s = self.state(cx);
            (
                s.visible_conversations()
                    .iter()
                    .map(|c| c.id)
                    .collect::<Vec<_>>(),
                s.selected,
            )
        };
        if ids.is_empty() {
            return;
        }
        let pos = cur
            .and_then(|c| ids.iter().position(|i| *i == c))
            .unwrap_or(0) as i32;
        let next = ids[(pos + delta).rem_euclid(ids.len() as i32) as usize];
        self.dispatch(Command::SelectConversation(next), cx);
    }

    pub(super) fn attach_files(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach file references".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            let paths = match result {
                Ok(Ok(Some(p))) => p,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(e)) => {
                    this.update(cx, |this, cx| {
                        this.show_toast(format!("File picker unavailable: {e}"), cx)
                    })
                    .ok();
                    return;
                }
            };
            this.update(cx, |this, cx| this.attach_paths(paths, cx))
                .ok();
        })
        .detach();
    }

    /// Attach files by path: from the picker, or dropped onto the window. Folders cannot be
    /// attached; they are skipped with a word about it. The files are read off the UI thread.
    pub(super) fn attach_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let (files, folders): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| !p.is_dir());
        if !folders.is_empty() {
            self.show_toast(
                if files.is_empty() {
                    "Folders cannot be attached; drop files instead".to_string()
                } else {
                    format!("Attached {} file(s); folders were skipped", files.len())
                },
                cx,
            );
        }
        if files.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let described = cx
                .background_spawn(async move {
                    files
                        .iter()
                        .map(|p| describe_attachment(p))
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                this.dispatch(Command::AddAttachments(described), cx)
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn copy_draft(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).text();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast("Draft copied to clipboard", cx);
    }

    // ------------------------------------------------------------------ resizing

    fn on_drag_move(&mut self, split: Split, x: f32, total: f32, cx: &mut Context<Self>) {
        match split {
            Split::Nav => self.live_nav = Some(x.clamp(180.0, 420.0)),
            Split::Inspector => {
                self.live_insp = Some(inspector_drag_width(x, total, self.nav_width(cx)))
            }
        }
        cx.notify();
    }

    fn end_drag(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(w) = self.live_nav.take() {
            self.dispatch(Command::SetNavWidth(w), cx);
        }
        if let Some(w) = self.live_insp.take() {
            self.dispatch(Command::SetInspectorWidth(w), cx);
        }
    }

    pub(super) fn divider(
        &self,
        id: &'static str,
        split: Split,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let border = cx.theme().colors.border;
        let accent = cx.theme().colors.accent;
        let marker = match split {
            Split::Nav => 0usize,
            Split::Inspector => 1,
        };
        div()
            .id(id)
            .w(px(DIVIDER_WIDTH))
            .h_full()
            .flex_none()
            .cursor_col_resize()
            .flex()
            .justify_center()
            .hover(move |s| s.bg(accent.opacity(0.25)))
            .on_drag(DragMarker(marker), |_, _, _, cx| cx.new(|_| DragGhost))
            .child(div().w(px(1.0)).h_full().bg(border))
    }
}

#[derive(Clone)]
struct DragMarker(usize);

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.offer_question(window, cx);
        let t = cx.theme().clone();
        let size = window.viewport_size();
        let width = f32::from(size.width);
        let nav_docked = self.nav_docked(window);
        let insp_docked = self.inspector_docked(window, cx);
        let nav_w = self.nav_width(cx);
        let insp_w = self.inspector_width(cx);
        let temp = self.temp_panel;

        let mut row = div().flex().flex_row().size_full().min_h_0();
        if nav_docked {
            row = row.child(
                div()
                    .w(px(nav_w))
                    .h_full()
                    .flex_none()
                    .child(self.render_nav(window, cx)),
            );
            row = row.child(self.divider("nav-divider", Split::Nav, cx));
        }
        row = row.child(self.render_center(window, cx));
        if insp_docked {
            row = row.child(self.divider("inspector-divider", Split::Inspector, cx));
            row = row.child(
                div()
                    .w(px(insp_w))
                    .h_full()
                    .flex_none()
                    .child(self.render_inspector(window, cx)),
            );
        }

        let scrim = t.colors.scrim;
        let mut root = div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.root_focus)
            .relative()
            .size_full()
            .bg(t.colors.bg_app)
            .text_color(t.colors.text)
            .text_size(t.ui_size())
            .font_family(t.ui_font())
            .on_action(cx.listener(Self::on_open_palette))
            .on_action(cx.listener(Self::on_new_conversation))
            .on_action(cx.listener(Self::on_toggle_nav))
            .on_action(cx.listener(Self::on_toggle_inspector))
            .on_action(cx.listener(Self::on_focus_composer))
            .on_action(cx.listener(Self::on_focus_transcript))
            .on_action(cx.listener(Self::on_prefs))
            .on_action(cx.listener(Self::on_rename))
            .on_action(cx.listener(Self::on_cancel))
            .on_action(cx.listener(Self::on_queue))
            .on_action(cx.listener(Self::on_steer))
            .on_action(cx.listener(Self::on_jump))
            .on_action(cx.listener(Self::on_model_menu))
            .on_action(cx.listener(Self::on_attach))
            .on_action(cx.listener(Self::on_open_project))
            .on_action(cx.listener(Self::on_answer_question))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_prev))
            .on_action(cx.listener(Self::on_close_overlay))
            .on_action(cx.listener(Self::on_menu_up))
            .on_action(cx.listener(Self::on_menu_down))
            .on_action(cx.listener(Self::on_menu_confirm))
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_log_stats))
            .on_drag_move::<DragMarker>(cx.listener(
                move |this, ev: &gpui::DragMoveEvent<DragMarker>, _, cx| {
                    let x = f32::from(ev.event.position.x);
                    match ev.drag(cx).0 {
                        0 => this.on_drag_move(Split::Nav, x, width, cx),
                        _ => this.on_drag_move(Split::Inspector, x, width, cx),
                    }
                },
            ))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::end_drag))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::end_drag))
            .child(row);

        if let Some(panel) = temp {
            let content = match panel {
                Panel::Nav => self.render_nav(window, cx).into_any_element(),
                Panel::Inspector => self.render_inspector(window, cx).into_any_element(),
            };
            let pw = (if panel == Panel::Nav { nav_w } else { insp_w })
                .min(width - 48.0)
                .max(260.0);
            let drawer = div()
                .id("temp-panel")
                .key_context("Panel")
                .track_focus(&self.panel_focus)
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(pw))
                .bg(t.colors.bg_pane)
                .border_color(t.colors.border_strong)
                .when(panel == Panel::Nav, |d| d.left_0().border_r_1())
                .when(panel == Panel::Inspector, |d| d.right_0().border_l_1())
                .shadow_lg()
                .occlude()
                .child(content);
            root = root.child(
                deferred(
                    div()
                        .absolute()
                        .inset_0()
                        .child(
                            div()
                                .id("panel-scrim")
                                .absolute()
                                .inset_0()
                                .bg(scrim)
                                .occlude()
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.close_panel(window, cx)),
                                ),
                        )
                        .child(drawer),
                )
                .with_priority(5),
            );
        }

        if self.overlay != Overlay::None {
            root = root.child(deferred(self.render_overlay(window, cx)).with_priority(10));
        }
        if let Some(toast) = self.toast.clone() {
            root = root.child(
                deferred(
                    div()
                        .absolute()
                        .bottom(px(24.0))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            super::controls::elevated(cx)
                                .px(px(14.0))
                                .py(px(8.0))
                                .child(toast),
                        ),
                )
                .with_priority(20),
            );
        }
        root
    }
}
