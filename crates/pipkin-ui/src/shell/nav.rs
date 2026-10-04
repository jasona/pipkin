use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Role, StatefulInteractiveElement,
    Styled, Window, div, px,
};
use pipkin_core::*;

use super::controls::*;
use super::workspace::{Overlay, Workspace};
use crate::theme::ActiveTheme;

pub fn relative_time(now: i64, at: i64) -> String {
    let d = (now - at).max(0);
    match d {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", d / 60),
        3600..=86399 => format!("{}h", d / 3600),
        86400..=604799 => format!("{}d", d / 86400),
        _ => format!("{}w", d / 604800),
    }
}

impl Workspace {
    pub(super) fn render_nav(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let (project_name, rows, selected, search_empty, now, any_in_project, query) = {
            let s = self.state(cx);
            let rows: Vec<_> = s
                .visible_conversations()
                .iter()
                .map(|cv| (cv.id, cv.title.clone(), cv.updated_at, cv.activity()))
                .collect();
            let any = s
                .conversations
                .iter()
                .any(|cv| Some(cv.project) == s.current_project().map(|p| p.id));
            (
                s.current_project()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "No project".into()),
                rows,
                s.selected,
                s.search.trim().is_empty(),
                s.now(),
                any,
                s.search.clone(),
            )
        };
        let this = cx.entity();
        let list: gpui::AnyElement = if rows.is_empty() {
            let (title, hint) = if !any_in_project {
                (
                    "No conversations yet".to_string(),
                    "Start one with New conversation.".to_string(),
                )
            } else if search_empty {
                ("Nothing here".to_string(), String::new())
            } else {
                (
                    format!("No conversations match “{query}”"),
                    "Try a different search.".to_string(),
                )
            };
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .px(px(12.0))
                .py(px(20.0))
                .child(div().text_color(c.text_muted).child(title))
                .child(
                    div()
                        .text_size(t.small_size())
                        .text_color(c.text_faint)
                        .child(hint),
                )
                .into_any_element()
        } else {
            div()
                .id("conversation-list")
                .role(Role::List)
                .aria_label("Conversations")
                .flex()
                .flex_col()
                .gap(px(2.0))
                .px(px(8.0))
                .overflow_y_scroll()
                .track_scroll(&self.nav_scroll)
                .flex_1()
                .min_h_0()
                .children(rows.into_iter().map(|(id, title, at, activity)| {
                    let is_sel = selected == Some(id);
                    let this = this.clone();
                    let mut row = menu_row(("conv", id.0 as usize), is_sel, cx)
                        .role(Role::ListItem)
                        .aria_label(title.clone())
                        .aria_selected(is_sel)
                        .justify_between()
                        .on_click(move |_, window, cx| {
                            this.update(cx, |this, cx| {
                                this.close_panel(window, cx);
                                this.dispatch(Command::SelectConversation(id), cx);
                            });
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(if is_sel { c.text } else { c.text_muted })
                                .child(title),
                        );
                    if let Some(a) = activity {
                        row = row.child(chip(a, c.accent, c.accent_bg, cx));
                    }
                    row.child(
                        div()
                            .flex_none()
                            .text_size(t.small_size())
                            .text_color(c.text_faint)
                            .child(relative_time(now, at)),
                    )
                }))
                .into_any_element()
        };

        let proj = {
            let this = cx.entity();
            Btn::new("project-switcher")
                .icon("folder")
                .label(project_name)
                .trailing_icon("chevron-down")
                .aria("Switch project")
                .on_click(move |window, cx| {
                    this.update(cx, |this, cx| {
                        this.open_overlay(Overlay::Project, window, cx)
                    })
                })
        };
        let new_btn = {
            let this = cx.entity();
            Btn::new("new-conversation")
                .icon("plus")
                .label("New conversation")
                .kind(BtnKind::Subtle)
                .on_click(move |window, cx| {
                    this.update(cx, |this, cx| {
                        this.close_panel(window, cx);
                        this.dispatch(Command::NewConversation, cx);
                        this.focus_composer(window, cx);
                    })
                })
        };
        let prefs_btn = {
            let this = cx.entity();
            Btn::new("open-prefs")
                .icon("settings")
                .aria("Preferences")
                .on_click(move |window, cx| {
                    this.update(cx, |this, cx| this.open_overlay(Overlay::Prefs, window, cx))
                })
        };
        let palette_btn = {
            let this = cx.entity();
            Btn::new("open-palette")
                .icon("command")
                .aria("Command palette")
                .on_click(move |window, cx| {
                    this.update(cx, |this, cx| {
                        this.open_overlay(Overlay::Palette, window, cx)
                    })
                })
        };

        div()
            .id("navigation")
            .role(Role::Navigation)
            .aria_label("Navigation")
            .flex()
            .flex_col()
            .size_full()
            .bg(c.bg_pane)
            .gap(px(8.0))
            .py(px(8.0))
            .child(
                div()
                    .px(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(proj)
                    .child(new_btn)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(10.0))
                            .h(t.control_height())
                            .rounded(px(6.0))
                            .bg(c.bg_input)
                            .border_1()
                            .border_color(c.border)
                            .child(icon("search", px(14.0), c.text_faint))
                            .child(div().flex_1().min_w_0().child(self.nav_search.clone())),
                    ),
            )
            .child(list)
            .child(
                div()
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(prefs_btn)
                    .child(palette_btn),
            )
    }
}
