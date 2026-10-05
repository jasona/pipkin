use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Role, StatefulInteractiveElement,
    Styled, Window, div, prelude::*, px,
};
use pipkin_core::*;

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
        let (title, project, has_conv, demo, banner, storage_issue) = {
            let s = self.state(cx);
            (
                s.current().map(|c| c.title.clone()),
                s.current_project()
                    .map(|p| format!("{} · {}", p.name, p.path))
                    .unwrap_or_default(),
                s.current().is_some(),
                s.mode == Mode::Demo,
                s.connection.banner(),
                s.storage_issue.clone(),
            )
        };
        let this = cx.entity();
        let storage_strip = storage_issue.map(|message| {
            let this = this.clone();
            div().px(px(12.0)).pt(px(8.0)).flex_none().child(strip(
                cx,
                "triangle-alert",
                c.warning,
                "Saved data",
                &message,
                vec![
                    Btn::new("dismiss-storage-issue")
                        .icon("x")
                        .aria("Dismiss")
                        .compact()
                        .on_click(move |_, cx| {
                            this.update(cx, |t, cx| t.dispatch(Command::DismissStorageIssue, cx))
                        })
                        .into_any_element(),
                ],
            ))
        });
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
            .children(demo.then(|| chip("Demo · simulated agent", c.text_muted, c.bg_active, cx)))
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
                        .child(banner.unwrap_or_else(|| "Create one with Ctrl+N.".into())),
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
            .children(storage_strip)
            .child(body)
            .children(has_conv.then(|| self.render_bottom(window, cx)))
    }

    /// Run status, queue, and composer, anchored below the transcript.
    fn render_bottom(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let (
            run,
            queue,
            draft_save,
            attachments,
            avail,
            models,
            model_id,
            retry_hint,
            intent_error,
            pending_queue,
            real,
        ) = {
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
                cv.intent_error.clone(),
                cv.pending_queue.clone(),
                s.mode == Mode::Real,
            )
        };
        let _ = retry_hint;
        let no_models = models.is_empty();
        let read_only = self.state(cx).read_only.clone();
        let model_name = models
            .iter()
            .find(|m| Some(&m.id) == model_id.as_ref())
            .map(|m| m.name.clone())
            .unwrap_or_else(|| {
                if models.is_empty() {
                    "No models available".into()
                } else {
                    "Choose model".into()
                }
            });
        let this = cx.entity();

        // ---- not-sent strip: the text was not sent (or not queued) and is still in the composer
        let not_sent: Option<gpui::AnyElement> = intent_error.as_deref().map(|message| {
            strip(
                cx,
                "circle-alert",
                c.danger,
                "Not sent",
                message,
                vec![
                    Btn::new("dismiss-intent-error")
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
            )
        });

        // ---- status strip
        let status: Option<gpui::AnyElement> = match &run {
            // The engine has no model Pipkin can offer: say what to do about it.
            RunState::Idle if real && no_models && read_only.is_none() => Some(strip(
                cx,
                "circle-help",
                c.warning,
                "No model is ready",
                "Pi has no model it can use. Sign in or add an API key with Pi (run `pi`, then /login), then refresh the models.",
                vec![
                    Btn::new("refresh-models")
                        .icon("refresh-cw")
                        .label("Refresh models")
                        .kind(BtnKind::Subtle)
                        .compact()
                        .disabled(!avail.refresh_models)
                        .on_click({
                            let this = this.clone();
                            move |_, cx| {
                                this.update(cx, |t, cx| t.dispatch(Command::RefreshModels, cx))
                            }
                        })
                        .into_any_element(),
                ],
            )),
            // This build can read sessions but not run them: say so where sending is offered.
            RunState::Idle if read_only.is_some() => Some(strip(
                cx,
                "circle-help",
                c.text_muted,
                "Read-only",
                read_only.as_deref().unwrap_or_default(),
                vec![],
            )),
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
                "The app or connection stopped before the prompt was acknowledged. Pipkin is asking the engine what happened; the prompt is never resent automatically.",
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
        // The engine's queue, then what was sent and not yet confirmed. Only what the engine
        // holds can be removed.
        let queue_total = queue.len() + pending_queue.len();
        let queue_el = (queue_total > 0).then(|| {
            let row = |i: usize, mode: QueueMode, text: String, note: Option<&'static str>| {
                let label = match mode {
                    QueueMode::Steer => "Steer",
                    QueueMode::FollowUp => "Follow-up",
                };
                div()
                    .id(("queued", i))
                    .role(Role::ListItem)
                    .aria_label(format!("{label}: {text}"))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().text_color(c.text_faint).child(format!("{}.", i + 1)))
                    .child(
                        div()
                            .text_size(t.small_size())
                            .text_color(c.accent)
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(c.text_muted)
                            .child(text),
                    )
                    .children(note.map(|n| {
                        div()
                            .text_size(t.small_size())
                            .text_color(c.text_faint)
                            .child(n)
                    }))
            };
            let held = queue.len();
            div()
                .id("queue")
                .role(Role::List)
                .aria_label("Queued input")
                .flex()
                .flex_col()
                .gap(px(2.0))
                .py(px(6.0))
                .child(
                    div()
                        .text_size(t.small_size())
                        .text_color(c.text_faint)
                        .child(format!("Queued ({queue_total})")),
                )
                .children(queue.into_iter().enumerate().map(|(i, q)| {
                    let this = this.clone();
                    let qid = q.id;
                    row(i, q.mode, q.text, None).child(
                        Btn::new(("unqueue", qid.0 as usize))
                            .icon("x")
                            .compact()
                            .aria("Remove queued prompt")
                            .on_click(move |_, cx| {
                                this.update(cx, |t, cx| t.dispatch(Command::RemoveQueued(qid), cx))
                            }),
                    )
                }))
                .children(pending_queue.into_iter().enumerate().map(|(i, p)| {
                    let note = match p.state {
                        QueueSend::Sending => "Sending…",
                        QueueSend::Unknown => "Checking with the engine…",
                    };
                    row(held + i, p.mode, p.text, Some(note))
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
                        Btn::new("retry-save")
                            .icon("refresh-cw")
                            .label("Retry save")
                            .compact()
                            .disabled(!avail.retry_save)
                            .on_click(move |_, cx| {
                                this.update(cx, |t, cx| t.dispatch(Command::RetrySave, cx))
                            })
                    })
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
                    .disabled(no_models)
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
                    .children(not_sent)
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
