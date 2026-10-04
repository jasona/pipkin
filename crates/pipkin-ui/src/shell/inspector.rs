use std::rc::Rc;

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Role, StatefulInteractiveElement,
    Styled, Window, div, prelude::*, px, uniform_list,
};
use pipkin_core::*;

use super::controls::*;
use super::workspace::{Panel, Workspace};
use crate::theme::ActiveTheme;

impl Workspace {
    fn refresh_diff_rows(&mut self, cx: &gpui::App) {
        let key = {
            let s = self.state(cx);
            s.current().and_then(|c| {
                c.selected_change
                    .and_then(|i| c.changes.get(i).map(|f| (c.id, i, f.added, f.removed)))
            })
        };
        if self.diff_rows.key == key {
            return;
        }
        self.diff_rows.key = key;
        self.diff_rows.rows.clear();
        self.diff_rows.widest = 0;
        self.diff_rows.change = None;
        let Some((id, i, ..)) = key else { return };
        let change = self
            .state(cx)
            .conversation(id)
            .and_then(|c| c.changes.get(i))
            .cloned();
        if let Some(f) = change {
            let mut widest = 0usize;
            let mut best = 0usize;
            for (h, hunk) in f.hunks.iter().enumerate() {
                self.diff_rows.rows.push((h as u32, u32::MAX));
                for (l, line) in hunk.lines.iter().enumerate() {
                    let w = line.text.chars().count();
                    if w > widest {
                        widest = w;
                        best = self.diff_rows.rows.len();
                    }
                    self.diff_rows.rows.push((h as u32, l as u32));
                }
            }
            self.diff_rows.widest = best;
            self.diff_rows.change = Some(Rc::new(f));
        }
    }

    pub(super) fn render_inspector(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.refresh_diff_rows(cx);
        let t = cx.theme().clone();
        let c = &t.colors;
        let (changes, selected) = {
            let s = self.state(cx);
            match s.current() {
                Some(cv) => (
                    cv.changes
                        .iter()
                        .map(|f| (f.path.clone(), f.added, f.removed))
                        .collect::<Vec<_>>(),
                    cv.selected_change,
                ),
                None => (vec![], None),
            }
        };
        let this = cx.entity();
        let temp = self.temp_panel == Some(Panel::Inspector);

        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(48.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(c.border)
            .child(icon("file-diff", px(16.0), c.text_muted))
            .child(
                div()
                    .id("inspector-title")
                    .role(Role::Heading)
                    .aria_label("Workspace changes, demo")
                    .flex_1()
                    .truncate()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Workspace changes · Demo"),
            )
            .when(temp, |d| {
                let this = this.clone();
                d.child(
                    Btn::new("close-inspector")
                        .icon("x")
                        .aria("Close inspector")
                        .on_click(move |window, cx| {
                            this.update(cx, |t, cx| t.close_panel(window, cx))
                        }),
                )
            });

        let body: gpui::AnyElement = if changes.is_empty() {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .px(px(24.0))
                .child(div().text_color(c.text_muted).child("No changes yet"))
                .child(
                    div()
                        .text_size(t.small_size())
                        .text_color(c.text_faint)
                        .child("Files Pi edits in this conversation appear here with their diffs."),
                )
                .into_any_element()
        } else {
            let list = div()
                .id("changed-files")
                .role(Role::List)
                .aria_label("Changed files")
                .flex()
                .flex_col()
                .flex_none()
                .max_h(px(180.0 * t.scale))
                .overflow_y_scroll()
                .track_scroll(&self.changes_scroll)
                .p(px(6.0))
                .gap(px(2.0))
                .children(changes.iter().enumerate().map(|(i, (path, add, rem))| {
                    let this = this.clone();
                    let sel = selected == Some(i);
                    menu_row(("change", i), sel, cx)
                        .role(Role::ListItem)
                        .aria_label(format!("{path}, {add} added, {rem} removed"))
                        .aria_selected(sel)
                        .on_click(move |_, _, cx| {
                            this.update(cx, |t, cx| t.dispatch(Command::SelectChange(i), cx))
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_ellipsis_start()
                                .font_family(t.mono_font())
                                .text_size(t.small_size())
                                .child(path.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(t.small_size())
                                .text_color(c.diff_add_text)
                                .child(format!("+{add}")),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(t.small_size())
                                .text_color(c.diff_remove_text)
                                .child(format!("−{rem}")),
                        )
                }));
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(list)
                .child(div().h(px(1.0)).w_full().bg(c.border))
                .child(self.render_diff(cx))
                .into_any_element()
        };

        div()
            .id("inspector")
            .role(Role::Complementary)
            .aria_label("Changes inspector")
            .flex()
            .flex_col()
            .size_full()
            .bg(c.bg_pane)
            .child(header)
            .child(body)
    }

    fn render_diff(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let Some(change) = self.diff_rows.change.clone() else {
            return div().flex_1().into_any_element();
        };
        let rows: Rc<Vec<(u32, u32)>> = Rc::new(self.diff_rows.rows.clone());
        let count = rows.len();
        let lh = t.code_line_height();
        let (add_bg, rem_bg, hunk_bg) = (
            t.colors.diff_add_bg,
            t.colors.diff_remove_bg,
            t.colors.diff_hunk_bg,
        );
        let (add_fg, rem_fg, faint, text, muted) = (
            t.colors.diff_add_text,
            t.colors.diff_remove_text,
            t.colors.text_faint,
            t.colors.text,
            t.colors.text_muted,
        );
        let mono = t.mono_font();
        let size = t.code_size();
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .h(px(32.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(t.mono_font())
                    .text_size(t.small_size())
                    .child(change.path.clone()),
            )
            .child(
                div()
                    .text_size(t.small_size())
                    .text_color(add_fg)
                    .child(format!("+{}", change.added)),
            )
            .child(
                div()
                    .text_size(t.small_size())
                    .text_color(rem_fg)
                    .child(format!("−{}", change.removed)),
            );
        let list = uniform_list("diff-rows", count, {
            let change = change.clone();
            move |range, _window, _cx| {
                range
                    .map(|ix| {
                        let (h, l) = rows[ix];
                        let hunk = &change.hunks[h as usize];
                        if l == u32::MAX {
                            return div()
                                .flex()
                                .h(lh)
                                .min_w_full()
                                .bg(hunk_bg)
                                .px(px(8.0))
                                .text_color(muted)
                                .font_family(mono.clone())
                                .text_size(size)
                                .whitespace_nowrap()
                                .child(hunk.header.clone())
                                .into_any_element();
                        }
                        let line = &hunk.lines[l as usize];
                        let (bg, fg, sign) = match line.kind {
                            DiffKind::Add => (Some(add_bg), add_fg, "+"),
                            DiffKind::Remove => (Some(rem_bg), rem_fg, "-"),
                            DiffKind::Context => (None, text, " "),
                        };
                        let num = |n: Option<u32>| {
                            div()
                                .w(px(44.0))
                                .flex_none()
                                .text_color(faint)
                                .text_right()
                                .pr(px(6.0))
                                .child(n.map(|n| n.to_string()).unwrap_or_default())
                        };
                        div()
                            .flex()
                            .h(lh)
                            .min_w_full()
                            .when_some(bg, |d, b| d.bg(b))
                            .font_family(mono.clone())
                            .text_size(size)
                            .whitespace_nowrap()
                            .child(num(line.old_no))
                            .child(num(line.new_no))
                            .child(div().w(px(16.0)).flex_none().text_color(fg).child(sign))
                            .child(div().text_color(fg).pr(px(12.0)).child(line.text.clone()))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }
        })
        .with_horizontal_sizing_behavior(gpui::ListHorizontalSizingBehavior::Unconstrained)
        .with_width_from_item(Some(self.diff_rows.widest))
        .track_scroll(&self.diff_scroll)
        .size_full();
        div()
            .id("diff")
            .role(Role::Document)
            .aria_label(format!("Diff for {}", change.path))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(div().flex_1().min_h_0().child(list))
            .into_any_element()
    }
}
