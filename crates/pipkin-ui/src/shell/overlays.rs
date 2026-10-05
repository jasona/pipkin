use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Role, StatefulInteractiveElement,
    Styled, Window, div, prelude::*, px,
};
use pipkin_core::*;

use super::commands::{self, Cmd};
use super::controls::*;
use super::workspace::{Overlay, Workspace};
use crate::model::DemoControls;
use crate::theme::ActiveTheme;

impl Workspace {
    fn palette_items(&self, cx: &gpui::App) -> Vec<Cmd> {
        let q = self.overlay_input.read(cx).text();
        let demo = cx.try_global::<DemoControls>();
        let mut scored: Vec<(usize, Cmd)> = commands::build(self.state(cx), demo)
            .into_iter()
            .filter_map(|c| {
                commands::fuzzy_score(q.trim(), &format!("{} {}", c.group, c.title)).map(|s| (s, c))
            })
            .collect();
        scored.sort_by_key(|(s, _)| *s);
        scored.into_iter().map(|(_, c)| c).collect()
    }

    pub(super) fn overlay_len(&self, cx: &gpui::App) -> usize {
        match self.overlay {
            Overlay::Palette => self.palette_items(cx).len(),
            Overlay::Model => self.state(cx).models.len(),
            Overlay::Project => self.state(cx).projects.len(),
            Overlay::Question => match self.waiting_question(cx, true) {
                Some(q) => match q.kind {
                    UiRequestKind::Select => q.items.len(),
                    UiRequestKind::Confirm => 2,
                    UiRequestKind::Input => 0,
                },
                None => 0,
            },
            _ => 0,
        }
    }

    pub(super) fn confirm_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sel = self.overlay_sel;
        match self.overlay {
            Overlay::Palette => {
                let items = self.palette_items(cx);
                if let Some(cmd) = items.get(sel) {
                    if !cmd.enabled {
                        return;
                    }
                    self.close_overlay(window, cx);
                    self.run_command(&cmd.run, window, cx);
                }
            }
            Overlay::Model => {
                if let Some(m) = self.state(cx).models.get(sel).map(|m| m.id.clone()) {
                    self.dispatch(Command::SetModel(m), cx);
                    self.close_overlay(window, cx);
                }
            }
            Overlay::Project => {
                if let Some(p) = self.state(cx).projects.get(sel).map(|p| p.id) {
                    self.dispatch(Command::SelectProject(p), cx);
                    self.close_overlay(window, cx);
                }
            }
            Overlay::Question => {
                let Some(q) = self.waiting_question(cx, true) else {
                    return self.close_overlay(window, cx);
                };
                let answer = match q.kind {
                    UiRequestKind::Select => {
                        q.items.get(sel).map(|i| UiAnswer::Choice(i.value.clone()))
                    }
                    UiRequestKind::Confirm => Some(UiAnswer::Confirm(sel == 0)),
                    UiRequestKind::Input => {
                        Some(UiAnswer::Text(self.overlay_input.read(cx).text()))
                    }
                };
                if let Some(answer) = answer {
                    self.dispatch(Command::AnswerUiRequest { id: q.id, answer }, cx);
                    self.close_overlay(window, cx);
                }
            }
            _ => {}
        }
    }

    pub(super) fn render_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let overlay = self.overlay;
        let this = cx.entity();
        let transparent = matches!(overlay, Overlay::Model | Overlay::Project);
        let backdrop = div()
            .id("overlay-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .when(!transparent, |d| d.bg(c.scrim))
            .on_click({
                let this = this.clone();
                move |_, window, cx| this.update(cx, |t, cx| t.close_overlay(window, cx))
            });

        let panel: gpui::AnyElement = match overlay {
            Overlay::Palette => self.render_palette(cx).into_any_element(),
            Overlay::Model => self.render_menu_overlay(cx).into_any_element(),
            Overlay::Project => self.render_menu_overlay(cx).into_any_element(),
            Overlay::Rename(_) => self.render_rename(cx).into_any_element(),
            Overlay::Prefs => self.render_prefs(cx).into_any_element(),
            Overlay::Question => self.render_question(cx).into_any_element(),
            Overlay::None => div().into_any_element(),
        };
        let h = window.viewport_size().height;
        let positioned = match overlay {
            Overlay::Model => div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .pb(px(168.0))
                .child(panel),
            Overlay::Project => div().absolute().left(px(8.0)).top(px(52.0)).child(panel),
            _ => div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .pt(px((f32::from(h) * 0.16).max(40.0)))
                .child(panel),
        };
        div().absolute().inset_0().child(backdrop).child(positioned)
    }

    fn render_palette(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let items = self.palette_items(cx);
        let sel = self.overlay_sel.min(items.len().saturating_sub(1));
        let this = cx.entity();
        let empty = items.is_empty();
        elevated(cx)
            .id("palette")
            .role(Role::Dialog)
            .aria_label("Command palette")
            .w(px(560.0 * t.scale.max(1.0)))
            .max_w_full()
            .flex()
            .flex_col()
            .occlude()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(14.0))
                    .h(px(46.0))
                    .border_b_1()
                    .border_color(c.border)
                    .child(icon("search", px(16.0), c.text_faint))
                    .child(div().flex_1().child(self.overlay_input.clone())),
            )
            .child(
                div()
                    .id("palette-list")
                    .role(Role::ListBox)
                    .flex()
                    .flex_col()
                    .p(px(6.0))
                    .max_h(px(380.0))
                    .overflow_y_scroll()
                    .children(items.into_iter().enumerate().map(|(i, cmd)| {
                        let this = this.clone();
                        let enabled = cmd.enabled;
                        let mut row = menu_row(("cmd", i), i == sel, cx)
                            .role(Role::ListBoxOption)
                            .aria_label(cmd.title.clone())
                            .aria_selected(i == sel)
                            .when(!enabled, |d| d.opacity(0.45))
                            .on_click(move |_, window, cx| {
                                this.update(cx, |t, cx| {
                                    t.overlay_sel = i;
                                    t.confirm_selection(window, cx);
                                })
                            })
                            .child(div().w(px(16.0)).child(if cmd.checked {
                                icon("check", px(14.0), c.accent).into_any_element()
                            } else {
                                div().into_any_element()
                            }))
                            .child(div().flex_1().min_w_0().truncate().child(cmd.title.clone()))
                            .child(
                                div()
                                    .text_size(t.small_size())
                                    .text_color(c.text_faint)
                                    .child(cmd.group),
                            );
                        if let Some(k) = cmd.shortcut {
                            row = row.child(kbd(k, cx));
                        }
                        row
                    }))
                    .when(empty, |d| {
                        d.child(
                            div()
                                .px(px(12.0))
                                .py(px(16.0))
                                .text_color(c.text_muted)
                                .child("No matching commands"),
                        )
                    }),
            )
    }

    /// A question an extension asked: its choices, a yes/no, or a line of text.
    fn render_question(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let this = cx.entity();
        let question = self.waiting_question(cx, true);
        let Some(q) = question else {
            return elevated(cx)
                .id("question")
                .child("This question is no longer open.");
        };
        let (answering, error, now_ms) = {
            let s = self.state(cx);
            let conv = s.current();
            (
                conv.is_some_and(|c| c.ui_answering.contains(&q.id)),
                conv.and_then(|c| c.ui_error.clone()),
                s.now() * 1000,
            )
        };
        let sel = self.overlay_sel;
        let rows: Vec<(String, Option<String>)> = match q.kind {
            UiRequestKind::Select => q
                .items
                .iter()
                .map(|i| (i.label.clone(), i.description.clone()))
                .collect(),
            UiRequestKind::Confirm => vec![("Yes".into(), None), ("No".into(), None)],
            UiRequestKind::Input => vec![],
        };
        let time_left = q.deadline.map(|d| {
            let secs = ((d - now_ms) / 1000).max(0);
            if secs >= 90 {
                format!("Cancels itself in about {} min", (secs + 59) / 60)
            } else {
                format!("Cancels itself in {secs} s")
            }
        });
        let decline = {
            let this = this.clone();
            let id = q.id.clone();
            Btn::new("question-decline")
                .label("Decline")
                .kind(BtnKind::Subtle)
                .compact()
                .on_click(move |window, cx| {
                    this.update(cx, |t, cx| {
                        t.dispatch(Command::CancelUiRequest(id.clone()), cx);
                        t.close_overlay(window, cx);
                    })
                })
        };
        elevated(cx)
            .id("question")
            .key_context("Overlay")
            .track_focus(&self.menu_focus)
            .role(Role::Dialog)
            .aria_label(format!("Question from an extension: {}", q.title))
            .w(px(520.0 * t.scale.max(1.0)))
            .max_w_full()
            .p(px(14.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .occlude()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(q.title.clone()),
            )
            .children(q.message.clone().map(|m| {
                div()
                    .text_size(t.small_size())
                    .text_color(c.text_muted)
                    .child(m)
            }))
            .when(q.kind == UiRequestKind::Input, |d| {
                d.child(
                    div()
                        .px(px(10.0))
                        .h(px(34.0))
                        .flex()
                        .items_center()
                        .rounded(px(6.0))
                        .bg(c.bg_input)
                        .border_1()
                        .border_color(c.border_strong)
                        .child(div().flex_1().child(self.overlay_input.clone())),
                )
            })
            .when(!rows.is_empty(), |d| {
                d.child(
                    div()
                        .id("question-list")
                        .role(Role::ListBox)
                        .flex()
                        .flex_col()
                        .max_h(px(280.0))
                        .overflow_y_scroll()
                        .children(rows.into_iter().enumerate().map(|(i, (label, note))| {
                            let this = this.clone();
                            menu_row(("choice", i), i == sel, cx)
                                .role(Role::ListBoxOption)
                                .aria_label(label.clone())
                                .aria_selected(i == sel)
                                .on_click(move |_, window, cx| {
                                    this.update(cx, |t, cx| {
                                        t.overlay_sel = i;
                                        t.confirm_selection(window, cx);
                                    })
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(div().truncate().child(label))
                                        .children(note.map(|n| {
                                            div()
                                                .truncate()
                                                .text_size(t.small_size())
                                                .text_color(c.text_faint)
                                                .child(n)
                                        })),
                                )
                        })),
                )
            })
            .children(error.map(|e| {
                div()
                    .text_size(t.small_size())
                    .text_color(c.danger)
                    .child(format!("Not accepted: {e}"))
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(t.small_size())
                            .text_color(c.text_faint)
                            .child(if answering {
                                "Sending your answer…".to_owned()
                            } else {
                                time_left.unwrap_or_default()
                            }),
                    )
                    .child(decline)
                    .when(q.kind == UiRequestKind::Input, |d| {
                        let this = this.clone();
                        d.child(
                            Btn::new("question-send")
                                .label("Answer")
                                .kind(BtnKind::Primary)
                                .compact()
                                .disabled(answering)
                                .on_click(move |window, cx| {
                                    this.update(cx, |t, cx| t.confirm_selection(window, cx))
                                }),
                        )
                    }),
            )
    }

    fn render_menu_overlay(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let this = cx.entity();
        let model_mode = self.overlay == Overlay::Model;
        let sel = self.overlay_sel;
        let entries: Vec<(String, String, bool)> = {
            let s = self.state(cx);
            if model_mode {
                s.models
                    .iter()
                    .map(|m| {
                        (
                            m.name.clone(),
                            m.note.clone(),
                            Some(&m.id) == s.prefs.model.as_ref(),
                        )
                    })
                    .collect()
            } else {
                let cur = s.current_project().map(|p| p.id);
                s.projects
                    .iter()
                    .map(|p| (p.name.clone(), p.path.clone(), Some(p.id) == cur))
                    .collect()
            }
        };
        elevated(cx)
            .id("menu-overlay")
            .key_context("Overlay")
            .track_focus(&self.menu_focus)
            .role(Role::Menu)
            .aria_label(if model_mode { "Models" } else { "Projects" })
            .w(px(300.0 * t.scale.max(1.0)))
            .p(px(6.0))
            .flex()
            .flex_col()
            .occlude()
            .children(
                entries
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, note, checked))| {
                        let this = this.clone();
                        menu_row(("menu", i), i == sel, cx)
                            .role(Role::MenuItem)
                            .aria_label(name.clone())
                            .on_click(move |_, window, cx| {
                                this.update(cx, |t, cx| {
                                    t.overlay_sel = i;
                                    t.confirm_selection(window, cx);
                                })
                            })
                            .child(div().w(px(16.0)).child(if checked {
                                icon("check", px(14.0), c.accent).into_any_element()
                            } else {
                                div().into_any_element()
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(div().truncate().child(name))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(t.small_size())
                                            .text_color(c.text_faint)
                                            .child(note),
                                    ),
                            )
                    }),
            )
    }

    fn render_rename(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let this = cx.entity();
        let Overlay::Rename(id) = self.overlay else {
            return div().into_any_element();
        };
        let this2 = this.clone();
        elevated(cx)
            .id("rename")
            .role(Role::Dialog)
            .aria_label("Rename conversation")
            .w(px(420.0 * t.scale.max(1.0)))
            .max_w_full()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .occlude()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Rename conversation"),
            )
            .child(
                div()
                    .px(px(10.0))
                    .h(t.control_height())
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .bg(c.bg_input)
                    .border_1()
                    .border_color(c.accent)
                    .child(div().flex_1().child(self.overlay_input.clone())),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        Btn::new("rename-cancel")
                            .label("Cancel")
                            .on_click(move |window, cx| {
                                this.update(cx, |t, cx| t.close_overlay(window, cx))
                            }),
                    )
                    .child(
                        Btn::new("rename-save")
                            .label("Rename")
                            .kind(BtnKind::Primary)
                            .on_click(move |window, cx| {
                                this2.update(cx, |t, cx| {
                                    let title = t.overlay_input.read(cx).text();
                                    t.dispatch(Command::RenameConversation(id, title), cx);
                                    t.close_overlay(window, cx);
                                })
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_prefs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let prefs = self.state(cx).prefs.clone();
        let this = cx.entity();
        let seg = |id: &'static str,
                   label: &'static str,
                   on: bool,
                   cmd: Command,
                   this: gpui::Entity<Workspace>| {
            Btn::new(id)
                .label(label)
                .kind(if on { BtnKind::Subtle } else { BtnKind::Ghost })
                .selected(on)
                .on_click(move |_, cx| this.update(cx, |t, cx| t.dispatch(cmd.clone(), cx)))
        };
        let row = |label: &'static str, ctrls: Vec<Btn>| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(div().text_color(c.text_muted).child(label))
                .child(div().flex().gap(px(4.0)).children(ctrls))
        };
        elevated(cx)
            .id("prefs")
            .key_context("Overlay")
            .track_focus(&self.menu_focus)
            .role(Role::Dialog)
            .aria_label("Preferences")
            .w(px(440.0 * t.scale.max(1.0)))
            .max_w_full()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .occlude()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Preferences"),
            )
            .child(row(
                "Theme",
                vec![
                    seg(
                        "theme-dark",
                        "Dark",
                        prefs.theme == Theme::Dark,
                        Command::SetTheme(Theme::Dark),
                        this.clone(),
                    ),
                    seg(
                        "theme-light",
                        "Light",
                        prefs.theme == Theme::Light,
                        Command::SetTheme(Theme::Light),
                        this.clone(),
                    ),
                ],
            ))
            .child(row(
                "Text size",
                vec![
                    seg(
                        "size-small",
                        "Small",
                        prefs.text_size == TextSize::Small,
                        Command::SetTextSize(TextSize::Small),
                        this.clone(),
                    ),
                    seg(
                        "size-normal",
                        "Normal",
                        prefs.text_size == TextSize::Normal,
                        Command::SetTextSize(TextSize::Normal),
                        this.clone(),
                    ),
                    seg(
                        "size-large",
                        "Large",
                        prefs.text_size == TextSize::Large,
                        Command::SetTextSize(TextSize::Large),
                        this.clone(),
                    ),
                ],
            ))
            .child(row(
                "Reduced motion",
                vec![
                    seg(
                        "motion-off",
                        "Off",
                        !prefs.reduced_motion,
                        Command::SetReducedMotion(false),
                        this.clone(),
                    ),
                    seg(
                        "motion-on",
                        "On",
                        prefs.reduced_motion,
                        Command::SetReducedMotion(true),
                        this.clone(),
                    ),
                ],
            ))
            .child(div().flex().justify_end().child({
                let this = this.clone();
                Btn::new("prefs-close")
                    .label("Done")
                    .kind(BtnKind::Primary)
                    .on_click(move |window, cx| {
                        this.update(cx, |t, cx| t.close_overlay(window, cx))
                    })
            }))
    }
}
