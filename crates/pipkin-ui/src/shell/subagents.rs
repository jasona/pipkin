//! A focused task directory/detail view, separate from workspace diffs. Presentation is
//! prepared on model updates; rendering never parses task prompts or guesses progress.
use super::{controls::*, workspace::Workspace};
use crate::theme::ActiveTheme;
use gpui::{Context, IntoElement, ParentElement, Role, Styled, div, prelude::*, px};
use pipkin_core::{Command, ConversationId, SubagentInfo};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum InspectorTab {
    #[default]
    Changes,
    Subagents,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TaskStatus {
    Pending,
    Running,
    Waiting,
    Completing,
    Done,
    Failed,
    Aborted,
    Unknown,
}
impl TaskStatus {
    fn from_engine(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "waiting" => Self::Waiting,
            "completing" => Self::Completing,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "aborted" => Self::Aborted,
            _ => Self::Unknown,
        }
    }
    fn active(self) -> bool {
        matches!(
            self,
            Self::Pending | Self::Running | Self::Waiting | Self::Completing
        )
    }
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Running => "Running",
            Self::Waiting => "Waiting",
            Self::Completing => "Completing",
            Self::Done => "Completed",
            Self::Failed => "Failed",
            Self::Aborted => "Stopped",
            Self::Unknown => "Unknown",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::Done => "check",
            Self::Failed => "circle-alert",
            Self::Aborted => "square",
            Self::Running | Self::Completing => "loader-circle",
            _ => "clock",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct TaskRow {
    pub id: u64,
    pub title: String,
    pub task: String,
    pub summary: String,
    status: TaskStatus,
}

#[derive(Default)]
pub(super) struct SubagentRows {
    key: Option<(ConversationId, u64)>,
    source: Vec<SubagentInfo>,
    pub rows: Vec<TaskRow>,
    pub selected_row: Option<TaskRow>,
    pub total: usize,
    pub active: usize,
    pub completed: usize,
    pub attention: usize,
}
impl SubagentRows {
    pub fn refresh(
        &mut self,
        key: Option<(ConversationId, u64)>,
        children: &[SubagentInfo],
        selected: Option<u64>,
    ) {
        if self.key == key
            && self.source == children
            && self.selected_row.as_ref().map(|r| r.id) == selected
        {
            return;
        }
        self.selected_row = selected
            .and_then(|id| children.iter().find(|c| c.id == id))
            .map(task_row);
        if self.key == key && self.source == children {
            return;
        }
        self.key = key;
        self.source = children.to_vec();
        self.total = children.len();
        self.active = 0;
        self.completed = 0;
        self.attention = 0;
        for child in children {
            match TaskStatus::from_engine(&child.status) {
                status if status.active() => self.active += 1,
                TaskStatus::Done => self.completed += 1,
                TaskStatus::Failed | TaskStatus::Aborted => self.attention += 1,
                _ => {}
            }
        }
        // Active/attention tasks first, preserving engine order within each group. The bound
        // applies after grouping, so old finished work cannot hide every active task.
        let mut ordered = children.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|row| match TaskStatus::from_engine(&row.status) {
            status if status.active() => 0,
            TaskStatus::Failed | TaskStatus::Aborted => 1,
            _ => 2,
        });
        self.rows = ordered.into_iter().take(100).map(task_row).collect();
    }
}

fn task_row(child: &SubagentInfo) -> TaskRow {
    let description = if child.task.starts_with("You are ") {
        child
            .task
            .split_once('.')
            .map_or(child.task.as_str(), |(_, rest)| rest.trim())
    } else {
        &child.task
    };
    let mut summary: String = description.chars().take(120).collect();
    if description.chars().count() > 120 {
        summary.push('…');
    }
    let title = task_title(&child.task, child.id);
    if summary.trim_end_matches('.') == title {
        summary.clear();
    }
    TaskRow {
        id: child.id,
        title,
        task: child.task.clone(),
        summary,
        status: TaskStatus::from_engine(&child.status),
    }
}

// This is a UI-derived label from an engine task excerpt, never a claimed agent identity.
fn task_title(task: &str, id: u64) -> String {
    let first = task.trim().split(['\n', '.']).next().unwrap_or("").trim();
    let first = first.strip_prefix("You are ").unwrap_or(first);
    if !first.is_empty() && first.chars().count() <= 56 {
        first.to_owned()
    } else {
        format!("Subagent {id}")
    }
}

fn view_state(
    loading: bool,
    live: bool,
    failed: bool,
    viewed: bool,
    retained: bool,
) -> &'static str {
    if loading {
        "Loading activity…"
    } else if failed && retained {
        "Disconnected · retained preview"
    } else if failed {
        "Activity unavailable"
    } else if live {
        "Live activity"
    } else if viewed {
        "Snapshot"
    } else {
        "Activity unavailable"
    }
}

impl Workspace {
    pub(super) fn refresh_subagent_rows(&mut self, cx: &gpui::App) {
        let model = self.model.read(cx);
        let current = model.state.current();
        self.subagent_rows.refresh(
            current.map(|c| (c.id, c.generation)),
            current.map_or(&[], |c| c.subagents.children.as_slice()),
            current.and_then(|c| c.subagents.selected),
        );
    }

    fn task_status(&self, status: TaskStatus, cx: &gpui::App) -> impl IntoElement {
        let t = cx.theme();
        let c = &t.colors;
        let fg = match status {
            TaskStatus::Done => c.success,
            TaskStatus::Failed => c.danger,
            status if status.active() => c.warning,
            _ => c.text_muted,
        };
        div()
            .flex()
            .items_center()
            .gap(px(5.0))
            .flex_none()
            .text_size(t.small_size())
            .text_color(fg)
            .child(icon(status.icon(), px(14.0 * t.scale.max(1.0)), fg))
            .child(status.label())
    }

    pub(super) fn render_subagent_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = cx.theme().clone();
        let c = &t.colors;
        let model = self.model.read(cx);
        let state = model.state.current().map(|v| &v.subagents);
        let this = cx.entity();
        let mut panel = div()
            .id("subagents-inspector")
            .role(Role::Region)
            .aria_label("Subagent tasks")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0();
        let Some(state) = state else {
            return panel
                .child(
                    div()
                        .p(px(20.0))
                        .text_color(c.text_muted)
                        .child("Open a conversation to follow its subagents."),
                )
                .into_any_element();
        };
        if state.enabled.is_none() {
            return panel
                .child(div().p(px(20.0)).text_color(c.text_muted).child(
                    state.error.clone().unwrap_or_else(|| {
                        "This engine does not provide subagent activity.".into()
                    }),
                ))
                .into_any_element();
        }
        let summary = if self.subagent_rows.attention > 0 {
            format!(
                "{} active · {} completed · {} stopped or failed",
                self.subagent_rows.active,
                self.subagent_rows.completed,
                self.subagent_rows.attention
            )
        } else {
            format!(
                "{} active · {} completed",
                self.subagent_rows.active, self.subagent_rows.completed
            )
        };
        panel = panel.child(
            div()
                .id("subagents-summary")
                .role(Role::Status)
                .aria_label(summary.clone())
                .flex_none()
                .px(px(16.0))
                .py(px(12.0))
                .border_b_1()
                .border_color(c.border)
                .text_size(t.small_size())
                .text_color(c.text_muted)
                .child(summary),
        );
        if let Some(selected) = state.selected {
            // Selection remains engine-owned. A filtered/bounded directory must not replace it.
            let row = self.subagent_rows.selected_row.as_ref();
            let title = row.map_or_else(|| format!("Subagent {selected}"), |row| row.title.clone());
            let mut header = div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .p(px(16.0))
                .border_b_1()
                .border_color(c.border)
                .child(
                    Btn::new("subagent-return")
                        .icon("arrow-left")
                        .label("All tasks")
                        .aria("Back to all subagent tasks")
                        .on_click({
                            let this = this.clone();
                            move |_, cx| {
                                this.update(cx, |w, cx| {
                                    w.dispatch(Command::SelectSubagent(None), cx)
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .id("subagent-task-title")
                        .role(Role::Heading)
                        .aria_label(title.clone())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                );
            if let Some(row) = row {
                header = header.child(self.task_status(row.status, cx));
            }
            header = header.child(
                div()
                    .id("subagent-view-state")
                    .role(Role::Status)
                    .text_size(t.small_size())
                    .text_color(c.text_muted)
                    .child(view_state(
                        state.loading,
                        state.live,
                        state.error.is_some(),
                        state.viewed == Some(selected),
                        !state.previews.is_empty(),
                    )),
            );
            panel = panel.child(header);
            let mut activity = div()
                .id("subagent-transcript")
                .role(Role::List)
                .aria_label("Read-only subagent activity")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .track_scroll(&self.subagent_activity_scroll)
                .p(px(16.0))
                .gap(px(16.0));
            if let Some(row) = row {
                activity = activity.child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .text_size(t.small_size())
                        .child(div().text_color(c.text_muted).child("Task excerpt"))
                        .child(row.task.clone()),
                );
            }
            if let Some(error) = &state.error {
                activity = activity.child(
                    div()
                        .id("subagent-error")
                        .role(Role::Alert)
                        .text_color(c.warning)
                        .child(error.clone()),
                );
            }
            if state.previews.is_empty() {
                activity = activity.child(
                    div()
                        .id("subagent-empty-activity")
                        .role(Role::Status)
                        .text_color(c.text_muted)
                        .child(if state.loading {
                            "Reading this task’s activity…"
                        } else {
                            "No transcript entries available yet."
                        }),
                );
            }
            for entry in &state.previews {
                let tool = entry.speaker == "Tool";
                activity = activity.child(
                    div()
                        .id(("subagent-entry", entry.id.0 as usize))
                        .role(Role::ListItem)
                        .flex_none()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(t.small_size())
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(c.text_muted)
                                .child(entry.speaker),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .text_size(t.small_size())
                                .when(tool, |d| d.font_family(t.mono_font()))
                                .child(entry.text.clone()),
                        )
                        .when(entry.clipped, |d| {
                            d.child(
                                div()
                                    .text_size(t.small_size())
                                    .text_color(c.text_muted)
                                    .child("Preview shortened"),
                            )
                        }),
                );
            }
            panel = panel.child(activity);
        } else {
            if let Some(error) = &state.error {
                panel = panel.child(
                    div()
                        .id("subagent-directory-error")
                        .role(Role::Alert)
                        .p(px(16.0))
                        .text_color(c.warning)
                        .child(error.clone()),
                );
            }
            let mut directory = div()
                .id("subagent-directory")
                .role(Role::List)
                .aria_label("Subagent tasks; select a task to inspect activity")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&self.subagent_directory_scroll)
                .flex()
                .flex_col()
                .p(px(8.0));
            if self.subagent_rows.total == 0 {
                directory = directory.child(div().p(px(12.0)).flex().flex_col().gap(px(8.0)).child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("No delegated tasks yet"))
                    .child(div().text_size(t.small_size()).text_color(c.text_muted).child("When Pi delegates work, follow each task here. Ask it to use subagents for independent work.")));
            }
            for row in &self.subagent_rows.rows {
                let id = row.id;
                let click = this.clone();
                let key = this.clone();
                directory = directory.child(
                    menu_row(("subagent", id as usize), false, cx)
                        .role(Role::Button)
                        .aria_label(format!(
                            "{}, {}. Open activity. {}",
                            row.title,
                            row.status.label(),
                            row.task
                        ))
                        .flex_col()
                        .items_start()
                        .gap(px(8.0))
                        .py(px(12.0))
                        .w_full()
                        .min_w_0()
                        .on_click(move |_, _, cx| {
                            click.update(cx, |w, cx| {
                                w.subagent_activity_scroll = gpui::ScrollHandle::new();
                                w.dispatch(Command::SelectSubagent(Some(id)), cx);
                            });
                        })
                        .on_key_down(move |event, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                key.update(cx, |w, cx| {
                                    w.subagent_activity_scroll = gpui::ScrollHandle::new();
                                    w.dispatch(Command::SelectSubagent(Some(id)), cx);
                                });
                                cx.stop_propagation();
                            }
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .w_full()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(row.title.clone()),
                                )
                                .child(icon("chevron-right", px(14.0), c.text_muted)),
                        )
                        .child(self.task_status(row.status, cx))
                        .when(!row.summary.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_size(t.small_size())
                                    .text_color(c.text_muted)
                                    .min_w_0()
                                    .child(row.summary.clone()),
                            )
                        })
                        .child(separator(cx)),
                );
            }
            if self.subagent_rows.total > 100 {
                directory = directory.child(
                    div()
                        .p(px(12.0))
                        .text_size(t.small_size())
                        .text_color(c.warning)
                        .child(format!(
                            "Showing 100 of {} tasks; active work is listed first.",
                            self.subagent_rows.total
                        )),
                );
            }
            panel = panel.child(directory);
        }
        panel
            .child(
                div()
                    .id("subagents-read-only")
                    .flex_none()
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(c.border)
                    .text_size(t.small_size())
                    .text_color(c.text_muted)
                    .child(if state.selected.is_some() {
                        "Recent activity preview · read only. Messages go to the main conversation."
                    } else if state.enabled == Some(false) {
                        "New calls off in Preferences. Existing tasks continue."
                    } else {
                        "Select a task to follow its activity. New calls allowed."
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task(id: u64, text: &str, status: &str) -> SubagentInfo {
        SubagentInfo {
            id,
            task_id: id,
            call_id: format!("call-{id}"),
            task: text.into(),
            status: status.into(),
        }
    }
    #[test]
    fn labels_are_readable_unicode_safe_and_do_not_claim_engine_names() {
        assert_eq!(
            task_title("You are Counter A. Count to 20 slowly.", 2),
            "Counter A"
        );
        assert_eq!(
            task_title("Review the Rust tests. Do not edit.", 3),
            "Review the Rust tests"
        );
        assert_eq!(task_title(&"🦀".repeat(57), 4), "Subagent 4");
        assert_eq!(task_title("", 5), "Subagent 5");
        let row = task_row(&task(
            6,
            "You are Counter A. Count to 20 slowly.",
            "running",
        ));
        assert_eq!(row.summary, "Count to 20 slowly.");
        assert_eq!(row.task, "You are Counter A. Count to 20 slowly.");
        assert!(
            task_row(&task(7, "Review Rust tests.", "done"))
                .summary
                .is_empty()
        );
        let row = task_row(&task(8, &"🦀".repeat(160), "waiting"));
        assert_eq!(row.summary.chars().count(), 121);
    }
    #[test]
    fn every_engine_status_is_distinct_and_unknown_does_not_imply_success() {
        for (raw, label, active) in [
            ("pending", "Pending", true),
            ("running", "Running", true),
            ("waiting", "Waiting", true),
            ("completing", "Completing", true),
            ("done", "Completed", false),
            ("failed", "Failed", false),
            ("aborted", "Stopped", false),
            ("unexpected", "Unknown", false),
        ] {
            let status = TaskStatus::from_engine(raw);
            assert_eq!(status.label(), label);
            assert_eq!(status.active(), active);
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../assets/icons")
                    .join(format!("{}.svg", status.icon()))
                    .is_file()
            );
        }
        assert!(TaskStatus::from_engine("completing").active());
        assert!(!TaskStatus::from_engine("done").active());
        assert_eq!(TaskStatus::from_engine("anything").label(), "Unknown");
    }
    #[test]
    fn active_tasks_survive_bounds_and_counts_include_undisplayed_work() {
        let mut children = (1..=110)
            .map(|id| task(id, "Finished.", "done"))
            .collect::<Vec<_>>();
        children.push(task(111, "Running.", "running"));
        children.push(task(112, "Failed.", "failed"));
        let mut rows = SubagentRows::default();
        rows.refresh(Some((ConversationId(1), 2)), &children, Some(110));
        assert_eq!(
            (rows.total, rows.active, rows.completed, rows.attention),
            (112, 1, 110, 1)
        );
        assert_eq!(rows.selected_row.as_ref().unwrap().id, 110);
        assert_eq!(rows.rows[0].id, 111);
        assert_eq!(rows.rows[1].id, 112);
        assert_eq!(rows.rows.len(), 100);
        rows.refresh(Some((ConversationId(2), 1)), &[], None);
        assert!(rows.rows.is_empty());
        assert_eq!(rows.active, 0);
    }
    #[test]
    fn loading_live_snapshot_and_disconnection_are_not_conflated() {
        assert_eq!(
            view_state(true, false, false, false, false),
            "Loading activity…"
        );
        assert_eq!(view_state(false, true, false, true, true), "Live activity");
        assert_eq!(view_state(false, false, false, true, true), "Snapshot");
        assert_eq!(
            view_state(false, false, true, false, false),
            "Activity unavailable"
        );
        assert_eq!(
            view_state(false, true, true, true, true),
            "Disconnected · retained preview"
        );
    }
}
