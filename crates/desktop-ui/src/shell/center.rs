use desktop_core::*;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Role, StatefulInteractiveElement,
    Styled, Window, div, prelude::*, px,
};

use super::controls::*;
use super::workspace::{Overlay, Panel, Workspace};
use crate::theme::ActiveTheme;

fn fmt_size(n: u64) -> String {
    match n {
        0..=1023 => format!("{n} B"),
        1024..=1048575 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1048576.0),
    }
}

impl Workspace {
    pub(super) fn render_center(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let nav_docked = self.nav_docked(window);
        let insp_docked = self.inspector_docked(window, cx);
        let (title, project, has_conv) = {
            let s = self.state(cx);
            (
                s.current().map(|c| c.title.clone()),
                s.current_project()
                    .map(|p| format!("{} · {}", p.name, p.path))
                    .unwrap_or_default(),
                s.current().is_some(),
            )
        };
        let this = cx.entity();
        let nav_toggle = (!nav_docked).then(|| {
            let this = this.clone();
            Btn::new("toggle-nav")
                .icon("panel-left")
                .aria("Show navigation")
                .selected(self.temp_panel == Some(Panel::Nav))
                .on_click(move |window, cx| {
                    this.update(cx, |this, cx| {
                        if this.temp_panel == Some(Panel::Nav) {
                            this.close_panel(window, cx)
                        } else {
                            this.open_panel(Panel::Nav, window, cx)
                        }
                    })
                })
        });
        let insp_toggle = {
            Btn::new("toggle-inspector")
                .icon("panel-right")
                .aria("Toggle changes inspector")
                .selected(insp_docked || self.temp_panel == Some(Panel::Inspector))
                .on_click(move |window, cx| {
                    window.dispatch_action(Box::new(super::actions::ToggleInspector), cx)
                })
        };

        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(48.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(c.border)
            .children(nav_toggle)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("conv-title")
                            .role(Role::Heading)
                            .aria_label(
                                title
                                    .clone()
                                    .unwrap_or_else(|| "No conversation selected".into()),
                            )
                            .truncate()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(
                                title
                                    .clone()
                                    .unwrap_or_else(|| "No conversation selected".into()),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(t.small_size())
                            .text_color(c.text_faint)
                            .child(project),
                    ),
            )
            .child(chip(
                "Demo · simulated agent",
                c.text_muted,
                c.bg_active,
                cx,
            ))
            .child(insp_toggle);

        let body = if has_conv {
            div()
                .flex_1()
                .min_h_0()
                .child(self.transcript.clone())
                .into_any_element()
        } else {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(
                    div()
                        .text_color(c.text_muted)
                        .child("No conversation selected"),
                )
                .child(
                    div()
                        .text_size(t.small_size())
                        .text_color(c.text_faint)
                        .child("Create one with Ctrl+N."),
                )
                .into_any_element()
        };

        div()
            .id("conversation")
            .role(Role::Main)
            .aria_label("Conversation")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(c.bg_surface)
            .child(header)
            .child(body)
            .children(has_conv.then(|| self.render_bottom(window, cx)))
    }

    /// Run status, queue, and composer, anchored below the transcript.
    fn render_bottom(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let (run, queue, draft_save, attachments, avail, models, model_id, retry_hint) = {
            let s = self.state(cx);
            let cv = s.current().unwrap();
            (
                cv.run.clone(),
                cv.queue.clone(),
                cv.draft.save.clone(),
                cv.draft.attachments.clone(),
                s.availability(),
                s.models.clone(),
                s.prefs.model.clone(),
                cv.last_submission.is_some(),
            )
        };
        let _ = (retry_hint, models.len());
        let model_name = models
            .iter()
            .find(|m| Some(&m.id) == model_id.as_ref())
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "Choose model".into());
        let this = cx.entity();

        // ---- status strip
        let status: Option<gpui::AnyElement> = match &run {
            RunState::Idle | RunState::Submitting { .. } => None,
            RunState::Running { .. } => Some(strip(
                cx,
                "loader-circle",
                c.accent,
                "Working…",
                "Pi is working. You can steer, queue a follow-up, or stop.",
                vec![
                    Btn::new("stop-run")
                        .icon("square")
                        .label("Stop")
                        .kind(BtnKind::Danger)
                        .compact()
                        .disabled(!avail.cancel)
                        .on_click({
                            let this = this.clone();
                            move |_, cx| this.update(cx, |t, cx| t.dispatch(Command::Cancel, cx))
                        })
                        .into_any_element(),
                ],
            )),
            RunState::Stopping { .. } => Some(strip(
                cx,
                "loader-circle",
                c.warning,
                "Stopping…",
                "Waiting for confirmation that the run has stopped.",
                vec![],
            )),
            RunState::OutcomeUnknown { .. } => Some(strip(
                cx,
                "circle-help",
                c.warning,
                "Outcome unknown",
                "The connection dropped before the prompt was acknowledged. It will not be resent automatically.",
                vec![
                    Btn::new("check-status")
                        .icon("refresh-cw")
                        .label("Check status")
                        .kind(BtnKind::Subtle)
                        .compact()
                        .on_click({
                            let this = this.clone();
                            move |_, cx| {
                                this.update(cx, |t, cx| t.dispatch(Command::CheckStatus, cx))
                            }
                        })
                        .into_any_element(),
                ],
            )),
            RunState::Failed { message } => Some(strip(
                cx,
                "circle-alert",
                c.danger,
                "Run failed",
                message,
                vec![
                    Btn::new("retry")
                        .icon("refresh-cw")
                        .label("Retry")
                        .kind(BtnKind::Subtle)
                        .compact()
                        .disabled(!avail.retry)
                        .on_click({
                            let this = this.clone();
                            move |_, cx| this.update(cx, |t, cx| t.dispatch(Command::Retry, cx))
                        })
                        .into_any_element(),
                    Btn::new("dismiss-failure")
                        .icon("x")
                        .aria("Dismiss")
                        .compact()
                        .on_click({
                            let this = this.clone();
                            move |_, cx| {
                                this.update(cx, |t, cx| t.dispatch(Command::DismissFailure, cx))
                            }
                        })
                        .into_any_element(),
                ],
            )),
        };

        // ---- queue
        let queue_el = (!queue.is_empty()).then(|| {
            div()
                .id("queue")
                .role(Role::List)
                .aria_label("Queued follow-ups")
                .flex()
                .flex_col()
                .gap(px(2.0))
                .py(px(6.0))
                .child(
                    div()
                        .text_size(t.small_size())
                        .text_color(c.text_faint)
                        .child(format!("Queued follow-ups ({})", queue.len())),
                )
                .children(queue.into_iter().enumerate().map(|(i, q)| {
                    let this = this.clone();
                    let qid = q.id;
                    div()
                        .id(("queued", i))
                        .role(Role::ListItem)
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(div().text_color(c.text_faint).child(format!("{}.", i + 1)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(c.text_muted)
                                .child(q.text),
                        )
                        .child(
                            Btn::new(("unqueue", qid.0 as usize))
                                .icon("x")
                                .compact()
                                .aria("Remove queued prompt")
                                .on_click(move |_, cx| {
                                    this.update(cx, |t, cx| {
                                        t.dispatch(Command::RemoveQueued(qid), cx)
                                    })
                                }),
                        )
                }))
        });

        // ---- attachments
        let att_el = (!attachments.is_empty()).then(|| {
            div().flex().flex_wrap().gap(px(6.0)).pb(px(6.0)).children(
                attachments.into_iter().enumerate().map(|(i, a)| {
                    let this = this.clone();
                    let (fg, bg) = if a.error.is_some() {
                        (c.danger, c.danger_bg)
                    } else {
                        (c.text_muted, c.bg_active)
                    };
                    let label = match (&a.error, a.size) {
                        (Some(e), _) => format!("{} — {}", a.name, e),
                        (None, Some(sz)) => format!("{} · {}", a.name, fmt_size(sz)),
                        _ => a.name.clone(),
                    };
                    chip(label, fg, bg, cx).id(("att", i)).child(
                        Btn::new(("rm-att", i))
                            .icon("x")
                            .compact()
                            .aria(format!("Remove {}", a.name))
                            .on_click(move |_, cx| {
                                this.update(cx, |t, cx| {
                                    t.dispatch(Command::RemoveAttachment(i), cx)
                                })
                            }),
                    )
                }),
            )
        });

        // ---- save state line
        let save_line: Option<gpui::AnyElement> = match &draft_save {
            SaveState::Failed(e) => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .py(px(4.0))
                    .text_size(t.small_size())
                    .text_color(c.danger)
                    .child(icon("triangle-alert", px(14.0), c.danger))
                    .child(
                        div()
                            .flex_1()
                            .child(format!("Draft not saved: {e}. Your text is still here.")),
                    )
                    .child({
                        let this = this.clone();
                        Btn::new("copy-draft")
                            .icon("copy")
                            .label("Copy draft")
                            .compact()
                            .on_click(move |_, cx| this.update(cx, |t, cx| t.copy_draft(cx)))
                    })
                    .into_any_element(),
            ),
            _ => None,
        };
        let save_label = match draft_save {
            SaveState::Clean => "",
            SaveState::Dirty => "Unsaved",
            SaveState::Saving => "Saving…",
            SaveState::Saved => "Draft saved",
            SaveState::Failed(_) => "Not saved",
        };

        // ---- toolbar
        let running = matches!(run, RunState::Running { .. } | RunState::Submitting { .. });
        let primary: gpui::AnyElement = if running {
            Btn::new("steer")
                .icon("arrow-up")
                .label("Steer")
                .kind(BtnKind::Primary)
                .disabled(!avail.steer)
                .on_click({
                    let this = this.clone();
                    move |_, cx| this.update(cx, |t, cx| t.dispatch(Command::Steer, cx))
                })
                .into_any_element()
        } else {
            Btn::new("send")
                .icon("arrow-up")
                .label("Send")
                .kind(BtnKind::Primary)
                .disabled(!avail.submit)
                .on_click({
                    let this = this.clone();
                    move |_, cx| this.update(cx, |t, cx| t.dispatch(Command::Submit, cx))
                })
                .into_any_element()
        };
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .pt(px(6.0))
            .child({
                let this = this.clone();
                Btn::new("attach")
                    .icon("paperclip")
                    .aria("Attach files (Ctrl+O)")
                    .on_click(move |_, cx| this.update(cx, |t, cx| t.attach_files(cx)))
            })
            .child({
                let this = this.clone();
                Btn::new("model-menu")
                    .label(model_name)
                    .trailing_icon("chevron-up")
                    .aria("Choose model (Ctrl+M)")
                    .selected(self.overlay == Overlay::Model)
                    .on_click(move |window, cx| {
                        this.update(cx, |t, cx| t.open_overlay(Overlay::Model, window, cx))
                    })
            })
            .child(div().flex_1())
            .child(
                div()
                    .text_size(t.small_size())
                    .text_color(c.text_faint)
                    .child(save_label),
            )
            .when(running, |d| {
                let this = this.clone();
                d.child(
                    Btn::new("queue-btn")
                        .label("Queue")
                        .kind(BtnKind::Subtle)
                        .disabled(!avail.queue)
                        .on_click(move |_, cx| {
                            this.update(cx, |t, cx| t.dispatch(Command::QueueFollowUp, cx))
                        }),
                )
            })
            .child(primary);

        let focused_ring = self
            .composer
            .read(cx)
            .focus_handle(cx)
            .contains_focused(_window, cx);

        div()
            .flex_none()
            .px(px(16.0))
            .pb(px(12.0))
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(760.0 * t.scale.max(1.0)))
                    .flex()
                    .flex_col()
                    .children(status)
                    .children(queue_el)
                    .child(
                        div()
                            .id("composer-box")
                            .role(Role::Group)
                            .aria_label("Message composer")
                            .flex()
                            .flex_col()
                            .p(px(10.0))
                            .rounded(px(10.0))
                            .bg(c.bg_input)
                            .border_1()
                            .border_color(if focused_ring {
                                c.accent
                            } else {
                                c.border_strong
                            })
                            .children(att_el)
                            .child(self.composer.clone())
                            .child(toolbar),
                    )
                    .children(save_line),
            )
    }
}

fn strip(
    cx: &gpui::App,
    icon_name: &'static str,
    color: gpui::Hsla,
    title: &str,
    detail: &str,
    actions: Vec<gpui::AnyElement>,
) -> gpui::AnyElement {
    let t = cx.theme();
    div()
        .id("run-status")
        .role(Role::Status)
        .aria_label(format!("{title}. {detail}"))
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(12.0))
        .py(px(8.0))
        .mb(px(8.0))
        .rounded(px(8.0))
        .bg(t.colors.bg_active)
        .child(icon(icon_name, px(16.0), color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_color(color)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(t.small_size())
                        .text_color(t.colors.text_muted)
                        .child(detail.to_string()),
                ),
        )
        .children(actions)
        .into_any_element()
}
