//! `TranscriptView`: the virtualized, selectable conversation transcript.
//!
//! Layout decisions worth knowing:
//! * One `gpui::list` item per transcript item, plus item 0 which is a fixed-height header row
//!   ("Loading earlier messages…" / "Beginning of conversation"). Fixed height means history
//!   loading never shifts the viewport by itself.
//! * Each conversation keeps its own `ListState`, expansion set, selection and block cache, so
//!   switching away and back restores the scroll anchor.
//! * Code blocks **wrap** rather than scroll horizontally: wrapped text keeps hit testing and
//!   selection geometry exact, and long unbroken tokens still break at the column edge.
//! * No animation anywhere. Streaming and running states are static indicators.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, DispatchPhase, Element, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, GlobalElementId, InspectorElementId,
    InteractiveText, IntoElement, KeyBinding, LayoutId, ListAlignment, ListOffset, ListScrollEvent,
    ListState, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Render, Role, SharedString, Styled, StyledText, Subscription, WeakEntity, Window, actions, div,
    list, point, prelude::*, px, svg,
};
use pipkin_core::{
    Attachment, Command, ConversationId, Delivery, ItemId, ItemKind, Note, NoticeLevel, ToolCall,
    ToolStatus, TranscriptItem,
};

use super::blocks::{BaseText, BlockText, FrameStats, SharedRegistry, build_runs};
use super::document::{DocPos, Document, Selection, Step, TOOL_INPUT, TOOL_OUTPUT, word_range};
use super::markdown::{Block, BlockKind};
use crate::model::Model;
use crate::theme::{ActiveTheme, Theme};

actions!(
    transcript,
    [
        SelectAllText,
        CopySelection,
        ExtendLeft,
        ExtendRight,
        ExtendUp,
        ExtendDown,
        ExtendToStart,
        ExtendToEnd,
        ClearSelection,
    ]
);

const CONTEXT: &str = "Transcript";
const MAX_CONVERSATION_VIEWS: usize = 24;
const OVERDRAW: f32 = 900.0;
const A11Y_TEXT_LIMIT: usize = 2000;

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-a", SelectAllText, Some(CONTEXT)),
        KeyBinding::new("ctrl-c", CopySelection, Some(CONTEXT)),
        KeyBinding::new("shift-left", ExtendLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", ExtendRight, Some(CONTEXT)),
        KeyBinding::new("shift-up", ExtendUp, Some(CONTEXT)),
        KeyBinding::new("shift-down", ExtendDown, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-home", ExtendToStart, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-end", ExtendToEnd, Some(CONTEXT)),
        KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
    ]);
}

/// Numbers for the perf scorecard.
#[derive(Clone, Debug, Default)]
pub struct TranscriptStats {
    /// Rows built in the most recent frame (bounded by viewport + overdraw).
    pub mounted_rows: usize,
    pub cached_blocks: usize,
    /// Bytes of selected text (0 when there is no selection).
    pub selection_len: usize,
    pub following: bool,
    pub frames: usize,
    pub frame_p50: Option<Duration>,
    pub frame_p95: Option<Duration>,
}

struct ConvView {
    list: ListState,
    doc: Document,
    expanded: HashSet<ItemId>,
    selection: Option<Selection>,
    last_used: u64,
}

struct Drag {
    conv: ConversationId,
    pointer: gpui::Point<Pixels>,
}

pub struct TranscriptView {
    model: Entity<Model>,
    focus: FocusHandle,
    convs: HashMap<ConversationId, ConvView>,
    current: Option<ConversationId>,
    registry: SharedRegistry,
    frame_stats: Rc<RefCell<FrameStats>>,
    list_bounds: Rc<Cell<Bounds<Pixels>>>,
    mounted: Rc<Cell<usize>>,
    drag: Option<Drag>,
    autoscroll: f32,
    autoscroll_task: Option<gpui::Task<()>>,
    copied: Option<(ItemId, u32)>,
    last_following: bool,
    tick: u64,
    resolved: Option<super::document::Resolved>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<()> for TranscriptView {}

impl Focusable for TranscriptView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[derive(Clone)]
enum Click {
    Url(String),
    Change(usize),
}

struct RowCtx {
    theme: Theme,
    conv: ConversationId,
    change_paths: Vec<String>,
    expanded: bool,
}

impl TranscriptView {
    pub fn new(model: Entity<Model>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sub = cx.subscribe(&model, |this, _model, note: &Note, cx| {
            this.on_note(note, cx)
        });
        TranscriptView {
            model,
            focus: cx.focus_handle(),
            convs: HashMap::new(),
            current: None,
            registry: Rc::default(),
            frame_stats: Rc::default(),
            list_bounds: Rc::default(),
            mounted: Rc::default(),
            drag: None,
            autoscroll: 0.0,
            autoscroll_task: None,
            copied: None,
            last_following: true,
            tick: 0,
            resolved: None,
            _subscriptions: vec![sub],
        }
    }

    // ------------------------------------------------------------------ public API

    pub fn jump_to_latest(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(cv) = self.current.and_then(|c| self.convs.get(&c)) {
            cv.list.set_follow_mode(gpui::FollowMode::Tail);
            cv.list.scroll_to_end();
        }
        cx.notify();
    }

    pub fn stats(&self, cx: &App) -> TranscriptStats {
        let fs = self.frame_stats.borrow();
        let cv = self.current.and_then(|c| self.convs.get(&c));
        let selection_len = self.selection_text(cx).map(|t| t.len()).unwrap_or(0);
        TranscriptStats {
            mounted_rows: self.mounted.get(),
            cached_blocks: cv.map(|c| c.doc.cached_blocks()).unwrap_or(0),
            selection_len,
            following: cv.map(|c| c.list.is_following_tail()).unwrap_or(true),
            frames: fs.len(),
            frame_p50: fs.percentile(0.5),
            frame_p95: fs.percentile(0.95),
        }
    }

    pub fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text(cx).filter(|t| !t.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        let model = self.model.clone();
        let Some(conv) = model.read(cx).state.conversation(id) else {
            return;
        };
        if let Some(cv) = self.convs.get_mut(&id) {
            cv.selection = cv.doc.select_all(&conv.items, &cv.expanded);
        }
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(cv) = self.current.and_then(|c| self.convs.get_mut(&c)) {
            cv.selection = None;
        }
        cx.notify();
    }

    pub fn has_selection(&self) -> bool {
        self.current
            .and_then(|c| self.convs.get(&c))
            .is_some_and(|cv| cv.selection.is_some_and(|s| s.anchor != s.head))
    }

    /// Expand/collapse a tool row programmatically (also used by clicks).
    pub fn toggle_tool(&mut self, conv: ConversationId, item: ItemId, cx: &mut Context<Self>) {
        let model = self.model.clone();
        let ix = model
            .read(cx)
            .state
            .conversation(conv)
            .and_then(|c| Document::index_of(&c.items, item));
        if let Some(cv) = self.convs.get_mut(&conv) {
            if !cv.expanded.remove(&item) {
                cv.expanded.insert(item);
            }
            if let Some(ix) = ix {
                cv.list.remeasure_items(ix + 1..ix + 2);
            }
        }
        cx.notify();
    }

    pub fn is_expanded(&self, conv: ConversationId, item: ItemId) -> bool {
        self.convs
            .get(&conv)
            .is_some_and(|c| c.expanded.contains(&item))
    }

    /// Scroll position of a conversation as (list item index, pixel offset); item 0 is the header.
    pub fn scroll_anchor(&self, conv: ConversationId) -> Option<(usize, Pixels)> {
        self.convs.get(&conv).map(|c| {
            let o = c.list.logical_scroll_top();
            (o.item_ix, o.offset_in_item)
        })
    }

    pub fn list_state(&self, conv: ConversationId) -> Option<ListState> {
        self.convs.get(&conv).map(|c| c.list.clone())
    }

    pub fn selection(&self, conv: ConversationId) -> Option<Selection> {
        self.convs.get(&conv).and_then(|c| c.selection)
    }

    pub fn set_selection(
        &mut self,
        conv: ConversationId,
        sel: Option<Selection>,
        cx: &mut Context<Self>,
    ) {
        if let Some(cv) = self.convs.get_mut(&conv) {
            cv.selection = sel;
        }
        cx.notify();
    }

    pub fn selection_text(&self, cx: &App) -> Option<String> {
        let id = self.current?;
        let cv = self.convs.get(&id)?;
        let sel = cv.selection?;
        let conv = self.model.read(cx).state.conversation(id)?;
        Some(cv.doc.selected_text(&conv.items, &cv.expanded, &sel))
    }

    // --------------------------------------------------------------- list bookkeeping

    fn total_rows(&self, conv: ConversationId, cx: &App) -> usize {
        self.model
            .read(cx)
            .state
            .conversation(conv)
            .map_or(1, |c| c.items.len() + 1)
    }

    fn make_list(&mut self, conv: ConversationId, cx: &mut Context<Self>) -> ListState {
        let total = self.total_rows(conv, cx);
        let list = ListState::new(total, ListAlignment::Bottom, px(OVERDRAW));
        list.set_follow_mode(gpui::FollowMode::Tail);
        let weak: WeakEntity<Self> = cx.entity().downgrade();
        list.set_scroll_handler(move |event: &ListScrollEvent, _window, cx| {
            let following = event.is_following_tail;
            let weak = weak.clone();
            // Never update the view from inside layout: defer.
            cx.defer(move |cx| {
                weak.update(cx, |this, cx| {
                    if this.last_following != following {
                        this.last_following = following;
                        cx.notify();
                    }
                    this.maybe_load_older(cx);
                })
                .ok();
            });
        });
        list
    }

    fn ensure_conv(&mut self, conv: ConversationId, cx: &mut Context<Self>) {
        self.tick += 1;
        let total = self.total_rows(conv, cx);
        if !self.convs.contains_key(&conv) {
            if self.convs.len() >= MAX_CONVERSATION_VIEWS {
                let victim = self
                    .convs
                    .iter()
                    .filter(|(id, _)| Some(**id) != self.current)
                    .min_by_key(|(_, v)| v.last_used)
                    .map(|(id, _)| *id);
                if let Some(v) = victim {
                    self.convs.remove(&v);
                }
            }
            let list = self.make_list(conv, cx);
            self.convs.insert(
                conv,
                ConvView {
                    list,
                    doc: Document::default(),
                    expanded: HashSet::new(),
                    selection: None,
                    last_used: self.tick,
                },
            );
        }
        let cv = self.convs.get_mut(&conv).unwrap();
        cv.last_used = self.tick;
        if cv.list.item_count() != total {
            // Safety net if a note was missed: resync and follow the tail.
            cv.list.reset(total);
            cv.list.set_follow_mode(gpui::FollowMode::Tail);
            cv.list.scroll_to_end();
        }
    }

    fn on_note(&mut self, note: &Note, cx: &mut Context<Self>) {
        match note {
            Note::ItemsReset(c) => {
                let total = self.total_rows(*c, cx);
                if let Some(cv) = self.convs.get_mut(c) {
                    cv.list.reset(total);
                    cv.list.set_follow_mode(gpui::FollowMode::Tail);
                    cv.list.scroll_to_end();
                    cv.selection = None;
                    cv.doc = Document::default();
                }
            }
            Note::ItemsAppended(c, _) => {
                let total = self.total_rows(*c, cx);
                if let Some(cv) = self.convs.get_mut(c) {
                    let old = cv.list.item_count();
                    if total > old {
                        cv.list.splice(old..old, total - old);
                    }
                }
            }
            Note::ItemChanged(c, i) => {
                if let Some(cv) = self.convs.get(c)
                    && i + 1 < cv.list.item_count()
                {
                    cv.list.remeasure_items(i + 1..i + 2);
                }
            }
            Note::ItemsPrepended(c, _) => {
                let total = self.total_rows(*c, cx);
                if let Some(cv) = self.convs.get_mut(c) {
                    let top = cv.list.logical_scroll_top();
                    let old = cv.list.item_count();
                    if total > old {
                        let delta = total - old;
                        cv.list.splice(1..1, delta);
                        // Re-anchor on the same item at the same pixel offset.
                        let anchor = if top.item_ix >= 1 {
                            ListOffset {
                                item_ix: top.item_ix + delta,
                                offset_in_item: top.offset_in_item,
                            }
                        } else {
                            ListOffset {
                                item_ix: delta + 1,
                                offset_in_item: px(0.),
                            }
                        };
                        cv.list.scroll_to(anchor);
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn maybe_load_older(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        let Some(cv) = self.convs.get(&id) else {
            return;
        };
        if cv.list.logical_scroll_top().item_ix > 2 {
            return;
        }
        let wants = self
            .model
            .read(cx)
            .state
            .conversation(id)
            .is_some_and(|c| c.has_older && !c.loading_older && c.opened);
        if wants {
            self.model
                .update(cx, |m, cx| m.dispatch(Command::LoadOlder, cx));
        }
    }

    // ------------------------------------------------------------------- selection

    fn current_selection_ctx(&self) -> Option<(ConversationId, Selection)> {
        let id = self.current?;
        Some((id, self.convs.get(&id)?.selection?))
    }

    fn pos_from_hit(hit: (ItemId, u32, usize)) -> DocPos {
        DocPos {
            item: hit.0,
            block: hit.1,
            offset: hit.2,
        }
    }

    fn mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        window.focus(&self.focus, cx);
        let hit = self.registry.borrow().hit_test(ev.position);
        let Some(hit) = hit else {
            if let Some(cv) = self.convs.get_mut(&id) {
                cv.selection = None;
            }
            cx.notify();
            return;
        };
        let pos = Self::pos_from_hit(hit);
        let model = self.model.clone();
        let Some(conv) = model.read(cx).state.conversation(id) else {
            return;
        };
        let Some(cv) = self.convs.get_mut(&id) else {
            return;
        };
        match ev.click_count {
            0 | 1 => {
                if ev.modifiers.shift {
                    if let Some(sel) = cv.selection.as_mut() {
                        sel.head = pos;
                    } else {
                        cv.selection = Some(Selection::caret(pos));
                    }
                } else {
                    cv.selection = Some(Selection::caret(pos));
                }
                self.drag = Some(Drag {
                    conv: id,
                    pointer: ev.position,
                });
            }
            n => {
                let item = conv.items.iter().find(|i| i.id == pos.item);
                let blocks = item.map(|i| cv.doc.blocks(i, cv.expanded.contains(&i.id)));
                if let Some(block) = blocks.as_ref().and_then(|b| b.get(pos.block as usize)) {
                    let range = if n == 2 {
                        word_range(&block.text, pos.offset)
                    } else {
                        0..block.text.len()
                    };
                    cv.selection = Some(Selection {
                        anchor: DocPos {
                            offset: range.start,
                            ..pos
                        },
                        head: DocPos {
                            offset: range.end,
                            ..pos
                        },
                    });
                }
                self.drag = None;
            }
        }
        cx.notify();
    }

    fn mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        if ev.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            self.autoscroll = 0.0;
            return;
        }
        drag.pointer = ev.position;
        let conv = drag.conv;
        let bounds = self.list_bounds.get();
        self.autoscroll = if ev.position.y < bounds.top() {
            -(f32::from(bounds.top() - ev.position.y)).clamp(4.0, 48.0)
        } else if ev.position.y > bounds.bottom() {
            f32::from(ev.position.y - bounds.bottom()).clamp(4.0, 48.0)
        } else {
            0.0
        };
        self.extend_to_pointer(conv, ev.position, cx);
        if self.autoscroll != 0.0 {
            self.ensure_autoscroll(cx);
        }
    }

    fn extend_to_pointer(
        &mut self,
        conv: ConversationId,
        pointer: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let hit = self.registry.borrow().hit_test(pointer);
        if let (Some(hit), Some(cv)) = (hit, self.convs.get_mut(&conv))
            && let Some(sel) = cv.selection.as_mut()
        {
            sel.head = Self::pos_from_hit(hit);
        }
        cx.notify();
    }

    fn mouse_up(&mut self) {
        self.drag = None;
        self.autoscroll = 0.0;
    }

    fn ensure_autoscroll(&mut self, cx: &mut Context<Self>) {
        if self.autoscroll_task.is_some() {
            return;
        }
        self.autoscroll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let keep = this
                    .update(cx, |v, cx| {
                        let Some(drag) = v.drag.as_ref().filter(|_| v.autoscroll != 0.0) else {
                            v.autoscroll_task = None;
                            return false;
                        };
                        let (conv, pointer) = (drag.conv, drag.pointer);
                        if let Some(cv) = v.convs.get(&conv) {
                            cv.list.scroll_by(px(v.autoscroll));
                        }
                        v.extend_to_pointer(conv, pointer, cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    fn extend(&mut self, step: Step, cx: &mut Context<Self>) {
        let Some((id, sel)) = self.current_selection_ctx() else {
            return;
        };
        let model = self.model.clone();
        let Some(conv) = model.read(cx).state.conversation(id) else {
            return;
        };
        let registry = self.registry.clone();
        let Some(cv) = self.convs.get_mut(&id) else {
            return;
        };
        let mut head = None;
        // Vertical movement follows visual lines when the head block is mounted.
        if matches!(step, Step::BlockUp | Step::BlockDown)
            && let Some(hit) = registry.borrow().find(sel.head.item, sel.head.block)
            && let Some(p) = hit.layout.position_for_index(sel.head.offset)
        {
            let lh = hit.layout.line_height();
            let y = if step == Step::BlockUp {
                p.y - lh
            } else {
                p.y + lh
            };
            let b = hit.bounds;
            if y >= b.top() && y < b.bottom() {
                let (Ok(i) | Err(i)) = hit.layout.index_for_position(point(p.x, y));
                head = Some(DocPos {
                    offset: i.min(hit.text_len),
                    ..sel.head
                });
            }
        }
        let head =
            head.unwrap_or_else(|| cv.doc.move_pos(&conv.items, &cv.expanded, sel.head, step));
        if let Some(s) = cv.selection.as_mut() {
            s.head = head;
        }
        cx.notify();
    }

    fn copy_block(&mut self, text: String, key: (ItemId, u32), cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = Some(key);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            this.update(cx, |v, cx| {
                if v.copied == Some(key) {
                    v.copied = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

// ------------------------------------------------------------------------ rendering

fn icon(name: &str, size: f32, color: gpui::Hsla) -> gpui::Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

fn format_time(at: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(dt) = Local.timestamp_opt(at, 0).single() else {
        return String::new();
    };
    if dt.date_naive() == Local::now().date_naive() {
        dt.format("%H:%M").to_string()
    } else {
        dt.format("%b %-d, %H:%M").to_string()
    }
}

fn clip(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn stop_mouse_down(_: &MouseDownEvent, _: &mut Window, cx: &mut App) {
    cx.stop_propagation();
}

fn element_key(item: ItemId, block: u32) -> u64 {
    item.0.wrapping_mul(1_000_003).wrapping_add(block as u64)
}

impl TranscriptView {
    fn render_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        self.mounted.set(self.mounted.get() + 1);
        let Some(conv_id) = self.current else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let model = self.model.clone();
        let Some(conv) = model.read(cx).state.conversation(conv_id) else {
            return div().into_any_element();
        };
        if ix == 0 {
            return self.render_header_row(
                conv.has_older,
                conv.loading_older,
                conv.opened,
                &theme,
                cx,
            );
        }
        let item_ix = ix - 1;
        let Some(item) = conv.items.get(item_ix) else {
            return div().into_any_element();
        };
        let item = item.clone();
        let prev_author = item_ix
            .checked_sub(1)
            .map(|p| &conv.items[p].kind)
            .map(|k| match k {
                ItemKind::User { .. } => 1,
                ItemKind::Assistant { .. } | ItemKind::Tool(_) => 2,
                ItemKind::Notice { .. } => 3,
            });
        let ctx = RowCtx {
            theme,
            conv: conv_id,
            change_paths: conv.changes.iter().map(|c| c.path.clone()).collect(),
            expanded: self.is_expanded(conv_id, item.id),
        };
        let body = match &item.kind {
            ItemKind::User {
                text,
                attachments,
                delivery,
                steer,
            } => self.render_user(
                &item,
                item_ix,
                text,
                attachments,
                *delivery,
                *steer,
                &ctx,
                cx,
            ),
            ItemKind::Assistant { text, streaming } => self.render_assistant(
                &item,
                item_ix,
                text,
                *streaming,
                prev_author != Some(2),
                &ctx,
                cx,
            ),
            ItemKind::Tool(t) => self.render_tool(&item, item_ix, t, &ctx, cx),
            ItemKind::Notice { text, level } => {
                self.render_notice(&item, item_ix, text, *level, &ctx, cx)
            }
        };
        let scale = ctx.theme.scale;
        div()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(760.0 * scale + 48.0))
                    .px(px(24.0))
                    .py(px(5.0 * scale))
                    .child(body),
            )
            .into_any_element()
    }

    fn render_header_row(
        &self,
        has_older: bool,
        loading: bool,
        opened: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let base = div()
            .h(px(40.0))
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(theme.small_size())
            .font_family(theme.ui_font())
            .text_color(c.text_faint);
        if loading {
            base.child("Loading earlier messages…").into_any_element()
        } else if has_older && opened {
            let model = self.model.clone();
            let _ = cx;
            base.child(
                div()
                    .id("load-older")
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(c.bg_hover))
                    .on_mouse_down(MouseButton::Left, stop_mouse_down)
                    .on_click(move |_, _, cx| {
                        model.update(cx, |m, cx| m.dispatch(Command::LoadOlder, cx));
                    })
                    .child("Load earlier messages"),
            )
            .into_any_element()
        } else if opened {
            base.child("Beginning of conversation").into_any_element()
        } else {
            base.child("Opening conversation…").into_any_element()
        }
    }

    fn author_line(
        &self,
        label: &str,
        at: i64,
        status: Option<(String, gpui::Hsla)>,
        theme: &Theme,
    ) -> gpui::Div {
        let c = &theme.colors;
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .mb(px(3.0))
            .text_size(theme.small_size())
            .font_family(theme.ui_font())
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c.text_muted)
                    .child(label.to_string()),
            )
            .child(div().text_color(c.text_faint).child(format_time(at)));
        if let Some((text, color)) = status {
            row = row.child(div().text_color(color).child(text));
        }
        row
    }

    #[allow(clippy::too_many_arguments)]
    fn render_user(
        &self,
        item: &TranscriptItem,
        item_ix: usize,
        text: &str,
        attachments: &[Attachment],
        delivery: Delivery,
        steer: bool,
        ctx: &RowCtx,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &ctx.theme;
        let c = &theme.colors;
        let status = match delivery {
            Delivery::Sent => None,
            Delivery::Pending => Some(("Sending…".to_string(), c.text_faint)),
            Delivery::Unknown => Some(("Outcome unknown".to_string(), c.warning)),
            Delivery::Rejected => Some(("Not sent".to_string(), c.danger)),
        };
        let label = if steer { "You (steering)" } else { "You" };
        let blocks = self.blocks_for(ctx.conv, item, ctx.expanded);
        let mut card = div()
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(8.0))
            .bg(c.bg_surface)
            .border_1()
            .border_color(c.border);
        for (bi, block) in blocks.iter().enumerate() {
            card = card.child(self.block_element(ctx, item, item_ix, bi, block, cx));
        }
        if !attachments.is_empty() {
            let mut chips = div().flex().flex_wrap().gap(px(6.0)).mt(px(6.0));
            for a in attachments {
                let tint = if a.error.is_some() {
                    c.danger
                } else {
                    c.text_muted
                };
                let label = match &a.error {
                    Some(e) => format!("{} — {e}", a.name),
                    None => a.name.clone(),
                };
                chips = chips.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .px(px(6.0))
                        .h(px(22.0 * theme.scale))
                        .rounded(px(4.0))
                        .bg(c.bg_hover)
                        .text_size(theme.small_size())
                        .text_color(tint)
                        .child(icon("paperclip", 12.0, tint))
                        .child(label),
                );
            }
            card = card.child(chips);
        }
        let aria = format!("You: {}", clip(text, A11Y_TEXT_LIMIT));
        div()
            .id(ElementId::NamedInteger("msg".into(), item.id.0))
            .role(Role::Article)
            .aria_label(aria)
            .child(self.author_line(label, item.at, status, theme))
            .child(card)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_assistant(
        &self,
        item: &TranscriptItem,
        item_ix: usize,
        text: &str,
        streaming: bool,
        show_label: bool,
        ctx: &RowCtx,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &ctx.theme;
        let c = &theme.colors;
        let blocks = self.blocks_for(ctx.conv, item, ctx.expanded);
        let mut col = div().w_full().flex().flex_col();
        if show_label {
            let status = streaming.then(|| ("Writing…".to_string(), c.text_faint));
            col = col.child(self.author_line("Pi", item.at, status, theme));
        }
        for (bi, block) in blocks.iter().enumerate() {
            col = col.child(self.block_element(ctx, item, item_ix, bi, block, cx));
        }
        if streaming {
            // Static indicator: no animation, identical under reduced motion.
            col = col.child(
                div()
                    .mt(px(2.0))
                    .w(px(7.0))
                    .h(px(14.0 * theme.scale))
                    .rounded(px(1.0))
                    .bg(c.accent),
            );
        }
        let aria = format!("Pi: {}", clip(text, A11Y_TEXT_LIMIT));
        div()
            .id(ElementId::NamedInteger("msg".into(), item.id.0))
            .role(Role::Article)
            .aria_label(aria)
            .child(col)
            .into_any_element()
    }

    fn render_notice(
        &self,
        item: &TranscriptItem,
        item_ix: usize,
        text: &str,
        level: NoticeLevel,
        ctx: &RowCtx,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &ctx.theme;
        let c = &theme.colors;
        let (name, tint, bg) = match level {
            NoticeLevel::Info => ("circle-help", c.text_muted, c.bg_hover),
            NoticeLevel::Error => ("circle-alert", c.danger, c.danger_bg),
        };
        let blocks = self.blocks_for(ctx.conv, item, ctx.expanded);
        let mut body = div().flex_1().min_w_0().text_color(tint);
        for (bi, block) in blocks.iter().enumerate() {
            body = body.child(self.block_element(ctx, item, item_ix, bi, block, cx));
        }
        let label = match level {
            NoticeLevel::Info => format!("Notice: {}", clip(text, A11Y_TEXT_LIMIT)),
            NoticeLevel::Error => format!("Error: {}", clip(text, A11Y_TEXT_LIMIT)),
        };
        div()
            .id(ElementId::NamedInteger("msg".into(), item.id.0))
            .role(Role::Article)
            .aria_label(label)
            .flex()
            .items_start()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(bg)
            .child(div().mt(px(3.0)).child(icon(name, 14.0, tint)))
            .child(body)
            .into_any_element()
    }

    fn render_tool(
        &self,
        item: &TranscriptItem,
        item_ix: usize,
        tool: &ToolCall,
        ctx: &RowCtx,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &ctx.theme;
        let c = &theme.colors;
        let (status_icon, status_text, status_color) = match tool.status {
            ToolStatus::Running => ("loader-circle", "Running", c.text_muted),
            ToolStatus::Ok => ("check", "Done", c.success),
            ToolStatus::Failed => ("circle-alert", "Failed", c.danger),
        };
        let summary = tool.input.lines().next().unwrap_or("").trim().to_string();
        let file_ref = ctx
            .change_paths
            .iter()
            .position(|p| !p.is_empty() && tool.input.contains(p.as_str()));
        let conv = ctx.conv;
        let id = item.id;
        let expanded = ctx.expanded;
        let model = self.model.clone();
        let mut header = div()
            .id(ElementId::NamedInteger("tool".into(), id.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(28.0 * theme.scale.max(1.0)))
            .px(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(c.border)
            .cursor_pointer()
            .hover(|s| s.bg(c.bg_hover))
            .role(Role::Button)
            .aria_label(format!("Tool {}: {}", tool.name, status_text))
            .aria_expanded(expanded)
            .on_mouse_down(MouseButton::Left, stop_mouse_down)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_tool(conv, id, cx)))
            .child(icon(
                if expanded {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                14.0,
                c.text_faint,
            ))
            .child(icon("terminal", 14.0, c.text_muted))
            .child(
                div()
                    .font_family(theme.mono_font())
                    .text_size(theme.code_size())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c.text)
                    .child(tool.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font())
                    .text_size(theme.code_size())
                    .text_color(c.text_muted)
                    .child(summary),
            );
        if let Some(ix) = file_ref {
            header = header.child(
                div()
                    .id(ElementId::NamedInteger("tool-diff".into(), id.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(6.0))
                    .rounded(px(4.0))
                    .text_size(theme.small_size())
                    .text_color(c.accent)
                    .hover(|s| s.bg(c.accent_bg))
                    .role(Role::Button)
                    .aria_label("View changes for this file")
                    .on_mouse_down(MouseButton::Left, stop_mouse_down)
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        model.update(cx, |m, cx| m.dispatch(Command::SelectChange(ix), cx));
                    })
                    .child(icon("file-diff", 12.0, c.accent))
                    .child("Diff"),
            );
        }
        header = header.child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_size(theme.small_size())
                .text_color(status_color)
                .child(icon(status_icon, 12.0, status_color))
                .child(status_text),
        );
        let mut col = div().w_full().flex().flex_col().child(header);
        if expanded {
            let blocks = self.blocks_for(conv, item, true);
            let mut body = div().w_full().pl(px(22.0)).pt(px(4.0));
            if blocks.is_empty() {
                body = body.child(
                    div()
                        .text_size(theme.small_size())
                        .text_color(c.text_faint)
                        .child(if tool.status == ToolStatus::Running {
                            "Waiting for output…"
                        } else {
                            "No output"
                        }),
                );
            }
            for (bi, block) in blocks.iter().enumerate() {
                body = body.child(self.block_element(ctx, item, item_ix, bi, block, cx));
            }
            if tool.truncated {
                body = body.child(
                    div()
                        .text_size(theme.small_size())
                        .text_color(c.warning)
                        .child(format!(
                            "Output truncated — showing first {} KB of {} KB",
                            tool.output.len().div_ceil(1024),
                            tool.full_len.div_ceil(1024)
                        )),
                );
            }
            col = col.child(body);
        }
        div()
            .id(ElementId::NamedInteger("msg".into(), id.0))
            .role(Role::Article)
            .aria_label(format!("Tool {}: {}", tool.name, status_text))
            .child(col)
            .into_any_element()
    }

    fn blocks_for(
        &self,
        conv: ConversationId,
        item: &TranscriptItem,
        expanded: bool,
    ) -> super::document::Blocks {
        match self.convs.get(&conv) {
            Some(cv) => cv.doc.blocks(item, expanded),
            None => Document::default().blocks(item, expanded),
        }
    }

    fn block_element(
        &self,
        ctx: &RowCtx,
        item: &TranscriptItem,
        item_ix: usize,
        bi: usize,
        block: &Block,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &ctx.theme;
        let c = &theme.colors;
        let is_code = block.is_code();
        if matches!(block.kind, BlockKind::Rule) {
            return div()
                .my(px(8.0))
                .h(px(1.0))
                .w_full()
                .bg(c.border_strong)
                .into_any_element();
        }
        let resolved = self.resolved;
        let selection = resolved.and_then(|r| r.range_in(item_ix, bi as u32, block.text.len()));
        // Inline code that names a changed file, and links, are clickable.
        let mut file_refs: Vec<Range<usize>> = Vec::new();
        let mut clicks: Vec<(Range<usize>, Click)> = Vec::new();
        if !is_code {
            for s in &block.spans {
                if let Some(url) = &s.style.link {
                    clicks.push((s.range.clone(), Click::Url(url.clone())));
                } else if s.style.code {
                    let t = &block.text[s.range.clone()];
                    let hit = ctx
                        .change_paths
                        .iter()
                        .position(|p| !t.is_empty() && (p == t || p.ends_with(&format!("/{t}"))));
                    if let Some(ix) = hit {
                        file_refs.push(s.range.clone());
                        clicks.push((s.range.clone(), Click::Change(ix)));
                    }
                }
            }
        }
        let (family, size, line_height, color, weight) = match &block.kind {
            BlockKind::Code { .. } => (
                theme.mono_font(),
                theme.code_size(),
                theme.code_line_height(),
                c.text,
                FontWeight::NORMAL,
            ),
            BlockKind::Heading(n) => {
                let f = match n {
                    1 => 1.45,
                    2 => 1.25,
                    3 => 1.12,
                    _ => 1.0,
                };
                (
                    theme.ui_font(),
                    px(f32::from(theme.body_size()) * f),
                    px((f32::from(theme.body_size()) * f * 1.35).round()),
                    c.text,
                    FontWeight::SEMIBOLD,
                )
            }
            _ => (
                theme.ui_font(),
                theme.body_size(),
                theme.body_line_height(),
                if block.quote { c.text_muted } else { c.text },
                FontWeight::NORMAL,
            ),
        };
        let base = BaseText {
            family: family.clone(),
            mono: theme.mono_font(),
            color,
            weight,
        };
        let runs = build_runs(block, theme, &base, selection, &file_refs);
        let key = element_key(item.id, bi as u32);
        let styled = StyledText::new(SharedString::from(block.text.clone())).with_runs(runs);
        let layout = styled.layout().clone();
        let inner: AnyElement = if clicks.is_empty() {
            styled.into_any_element()
        } else {
            let ranges: Vec<Range<usize>> = clicks.iter().map(|(r, _)| r.clone()).collect();
            let actions: Vec<Click> = clicks.into_iter().map(|(_, a)| a).collect();
            let model = self.model.clone();
            InteractiveText::new(ElementId::NamedInteger("tb".into(), key), styled)
                .on_click(ranges, move |i, _, cx| match &actions[i] {
                    Click::Url(u) => cx.open_url(u),
                    Click::Change(ix) => {
                        let ix = *ix;
                        model.update(cx, |m, cx| m.dispatch(Command::SelectChange(ix), cx));
                    }
                })
                .into_any_element()
        };
        let text = BlockText::new(
            layout,
            inner,
            self.registry.clone(),
            item.id,
            item_ix,
            bi as u32,
            block.text.len(),
        );
        let text_box = div()
            .w_full()
            .font_family(family)
            .text_size(size)
            .line_height(line_height)
            .text_color(color)
            .child(text);
        match &block.kind {
            BlockKind::Code { lang } => {
                let label = match lang.as_deref() {
                    Some(l) if l == TOOL_INPUT => "Input".to_string(),
                    Some(l) if l == TOOL_OUTPUT => "Output".to_string(),
                    Some(l) => l.to_string(),
                    None => "text".to_string(),
                };
                let copied = self.copied == Some((item.id, bi as u32));
                let copy_text = block.text.clone();
                let ckey = (item.id, bi as u32);
                let button = div()
                    .id(ElementId::NamedInteger("copy".into(), key))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(6.0))
                    .h(px(22.0))
                    .rounded(px(4.0))
                    .text_size(theme.small_size())
                    .font_family(theme.ui_font())
                    .text_color(if copied { c.success } else { c.text_muted })
                    .cursor_pointer()
                    .hover(|s| s.bg(c.bg_hover))
                    .role(Role::Button)
                    .aria_label(if copied { "Copied" } else { "Copy code" })
                    .on_mouse_down(MouseButton::Left, stop_mouse_down)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.copy_block(copy_text.clone(), ckey, cx)
                    }))
                    .child(icon(
                        if copied { "check" } else { "copy" },
                        12.0,
                        if copied { c.success } else { c.text_muted },
                    ))
                    .child(if copied { "Copied" } else { "Copy" });
                div()
                    .w_full()
                    .my(px(5.0))
                    .rounded(px(6.0))
                    .bg(c.code_bg)
                    .border_1()
                    .border_color(c.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pl(px(10.0))
                            .pr(px(4.0))
                            .pt(px(3.0))
                            .child(
                                div()
                                    .text_size(theme.small_size())
                                    .font_family(theme.mono_font())
                                    .text_color(c.text_faint)
                                    .child(label),
                            )
                            .child(button),
                    )
                    .child(div().px(px(10.0)).pb(px(8.0)).child(text_box))
                    .into_any_element()
            }
            BlockKind::ListItem => {
                let depth = block.indent.max(1) as f32 - 1.0;
                let mut d = div().w_full().pl(px(depth * 20.0 + 4.0)).mb(px(2.0));
                if block.quote {
                    d = d.border_l_2().border_color(c.border_strong).pl(px(12.0));
                }
                d.child(text_box).into_any_element()
            }
            BlockKind::Heading(_) => div()
                .w_full()
                .mt(px(8.0))
                .mb(px(3.0))
                .child(text_box)
                .into_any_element(),
            _ => {
                let mut d = div().w_full().mb(px(6.0));
                if block.quote {
                    d = d.border_l_2().border_color(c.border_strong).pl(px(12.0));
                }
                d.child(text_box).into_any_element()
            }
        }
    }
}

impl Render for TranscriptView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let c = theme.colors.clone();
        self.mounted.set(0);

        let selected = self.model.read(cx).state.selected;
        if selected != self.current {
            self.current = selected;
            self.drag = None;
            self.autoscroll = 0.0;
            self.last_following = true;
        }
        let mut empty = true;
        let mut run_label = "Idle";
        self.resolved = None;
        if let Some(id) = self.current {
            self.ensure_conv(id, cx);
            let model = self.model.clone();
            if let Some(conv) = model.read(cx).state.conversation(id) {
                empty = conv.items.is_empty() && !conv.has_older;
                run_label = conv.run.label();
                if let Some(cv) = self.convs.get(&id) {
                    self.resolved = cv
                        .selection
                        .and_then(|s| cv.doc.resolve(&conv.items, &cv.expanded, &s));
                }
            }
            self.maybe_load_older_deferred(cx);
        }

        let mut root = div()
            .id("transcript")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .role(Role::Log)
            .aria_label("Conversation transcript")
            .relative()
            .size_full()
            .on_action(cx.listener(|this, _: &SelectAllText, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &CopySelection, _, cx| this.copy_selection(cx)))
            .on_action(cx.listener(|this, _: &ExtendLeft, _, cx| this.extend(Step::Left, cx)))
            .on_action(cx.listener(|this, _: &ExtendRight, _, cx| this.extend(Step::Right, cx)))
            .on_action(cx.listener(|this, _: &ExtendUp, _, cx| this.extend(Step::BlockUp, cx)))
            .on_action(cx.listener(|this, _: &ExtendDown, _, cx| this.extend(Step::BlockDown, cx)))
            .on_action(
                cx.listener(|this, _: &ExtendToStart, _, cx| this.extend(Step::DocStart, cx)),
            )
            .on_action(cx.listener(|this, _: &ExtendToEnd, _, cx| this.extend(Step::DocEnd, cx)))
            .on_action(cx.listener(|this, _: &ClearSelection, _, cx| this.clear_selection(cx)))
            .cursor(gpui::CursorStyle::IBeam);

        let following = self
            .current
            .and_then(|id| self.convs.get(&id))
            .map(|cv| cv.list.is_following_tail())
            .unwrap_or(true);

        match (self.current, empty) {
            (Some(id), false) if self.convs.contains_key(&id) => {
                let list_state = self.convs[&id].list.clone();
                let rows = list(
                    list_state,
                    cx.processor(|this, ix: usize, _window, cx| this.render_row(ix, cx)),
                )
                .size_full();
                root = root.child(div().size_full().child(FrameProbe {
                    child: Some(rows.into_any_element()),
                    view: cx.entity().downgrade(),
                    registry: self.registry.clone(),
                    stats: self.frame_stats.clone(),
                    bounds_out: self.list_bounds.clone(),
                    started: None,
                }));
            }
            _ => {
                let (title, hint) = if self.current.is_none() {
                    (
                        "No conversation selected",
                        "Choose a conversation or start a new one.",
                    )
                } else {
                    ("No messages yet", "Write a prompt below to begin.")
                };
                root = root.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(6.0))
                        .font_family(theme.ui_font())
                        .child(
                            div()
                                .text_size(theme.ui_size())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(c.text_muted)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(theme.small_size())
                                .text_color(c.text_faint)
                                .child(hint),
                        ),
                );
            }
        }

        if !following && !empty {
            root = root.child(
                div().absolute().bottom(px(14.0)).right(px(22.0)).child(
                    div()
                        .id("jump-to-latest")
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .h(px(30.0 * theme.scale.max(1.0)))
                        .px(px(12.0))
                        .rounded(px(15.0))
                        .bg(c.bg_elevated)
                        .border_1()
                        .border_color(c.border_strong)
                        .shadow_md()
                        .text_size(theme.small_size())
                        .font_family(theme.ui_font())
                        .text_color(c.text)
                        .cursor_pointer()
                        .hover(|s| s.bg(c.bg_active))
                        .role(Role::Button)
                        .aria_label("Jump to latest message")
                        .on_mouse_down(MouseButton::Left, stop_mouse_down)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.jump_to_latest(window, cx)),
                        )
                        .child(icon("arrow-down", 14.0, c.text))
                        .child("Jump to latest"),
                ),
            );
        }

        // Live run status for assistive technology: a status node, visually empty.
        root = root.child(
            div()
                .id("run-status")
                .role(Role::Status)
                .aria_label(format!("Run status: {run_label}"))
                .absolute()
                .size(px(1.0)),
        );
        root
    }
}

impl TranscriptView {
    /// Like `maybe_load_older`, but safe to call from render.
    fn maybe_load_older_deferred(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        let Some(cv) = self.convs.get(&id) else {
            return;
        };
        if cv.list.logical_scroll_top().item_ix > 2 {
            return;
        }
        let wants = self
            .model
            .read(cx)
            .state
            .conversation(id)
            .is_some_and(|c| c.has_older && !c.loading_older && c.opened);
        if wants {
            let weak = cx.entity().downgrade();
            cx.defer(move |cx| {
                weak.update(cx, |this, cx| this.maybe_load_older(cx)).ok();
            });
        }
    }
}

/// Wraps the list: records paint cost, clears the hit registry each frame and routes mouse
/// events (which must keep flowing while the pointer is outside the list during a drag).
struct FrameProbe {
    child: Option<AnyElement>,
    view: WeakEntity<TranscriptView>,
    registry: SharedRegistry,
    stats: Rc<RefCell<FrameStats>>,
    bounds_out: Rc<Cell<Bounds<Pixels>>>,
    started: Option<Instant>,
}

impl IntoElement for FrameProbe {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FrameProbe {
    type RequestLayoutState = ();
    type PrepaintState = Duration;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.as_mut().unwrap().request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Duration {
        let start = Instant::now();
        self.started = Some(start);
        self.registry.borrow_mut().clear();
        self.bounds_out.set(bounds);
        self.child.as_mut().unwrap().prepaint(window, cx);
        start.elapsed()
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        prepaint_cost: &mut Duration,
        window: &mut Window,
        cx: &mut App,
    ) {
        let start = Instant::now();
        // Registered before the children paint, so child handlers (copy buttons, tool headers)
        // run first in the bubble phase and can stop propagation.
        let view = self.view.clone();
        window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || ev.button != MouseButton::Left
                || !bounds.contains(&ev.position)
            {
                return;
            }
            view.update(cx, |v, cx| v.mouse_down(ev, window, cx)).ok();
        });
        let view = self.view.clone();
        window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble {
                view.update(cx, |v, cx| v.mouse_move(ev, cx)).ok();
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble {
                view.update(cx, |v, _| v.mouse_up()).ok();
            }
        });
        self.child.as_mut().unwrap().paint(window, cx);
        self.stats
            .borrow_mut()
            .record(*prepaint_cost + start.elapsed());
    }
}

#[cfg(test)]
mod tests;
