use std::ops::Range;

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext, px};
use pipkin_core::*;

use super::*;
use crate::theme::Theme as UiTheme;

fn project() -> Project {
    Project {
        id: ProjectId(1),
        name: "demo".into(),
        path: "/demo".into(),
    }
}

fn boot() -> Bootstrap {
    Bootstrap {
        projects: vec![project()],
        models: vec![ModelInfo {
            id: "m".into(),
            name: "M".into(),
            note: String::new(),
        }],
        conversations: vec![
            (ConversationId(1), ProjectId(1), "one".into(), 10),
            (ConversationId(2), ProjectId(1), "two".into(), 5),
        ],
        now: 100,
    }
}

fn item(i: u64) -> TranscriptItem {
    let kind = match i % 5 {
        0 => ItemKind::User {
            text: format!("prompt number {i} please look at src/lib.rs"),
            attachments: vec![],
            delivery: Delivery::Sent,
            steer: false,
        },
        1 | 2 => ItemKind::Assistant {
            text: format!(
                "Paragraph {i} with **bold** and `code`.\n\n{}\n\n```rust\nfn f{i}() {{ println!(\"hi\"); }}\n```\n\n- one\n- two",
                "A fairly long sentence that should wrap across several lines when laid out. "
                    .repeat(4)
            ),
            streaming: false,
        },
        3 => ItemKind::Tool(ToolCall {
            call_ref: None,
            call_id: None,
            name: "bash".into(),
            input: format!("cargo test --package p{i}"),
            output: "line one\nline two\nline three".repeat(3),
            truncated: false,
            full_len: 80,
            status: ToolStatus::Ok,
        }),
        _ => ItemKind::Assistant {
            text: format!("Short reply {i}."),
            streaming: false,
        },
    };
    TranscriptItem {
        id: ItemId(i),
        at: 1_700_000_000 + i as i64,
        kind,
    }
}

fn items(r: Range<u64>) -> Vec<TranscriptItem> {
    r.map(item).collect()
}

struct Harness {
    model: Entity<Model>,
    view: Entity<TranscriptView>,
}

fn open(
    model: &Entity<Model>,
    conv: u64,
    items: Vec<TranscriptItem>,
    has_older: bool,
    cx: &mut VisualTestContext,
) {
    cx.update(|_, cx| {
        model.update(cx, |m, cx| {
            let generation = m
                .state
                .conversation(ConversationId(conv))
                .unwrap()
                .generation;
            m.apply_event(
                BackendEvent {
                    conversation: ConversationId(conv),
                    generation,
                    op: None,
                    kind: EventKind::Opened {
                        items,
                        has_older,
                        changes: vec![],
                    },
                },
                cx,
            );
        });
    });
}

/// Force a real window frame (the view is the window's root).
fn frame(_h: &Harness, cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn setup(cx: &mut TestAppContext, n: u64) -> (Harness, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(UiTheme::new(
            pipkin_core::Theme::Dark,
            TextSize::Normal,
            false,
        ));
        init(cx);
    });
    let mut state = AppState::new(boot(), Prefs::default());
    state.dispatch(Command::SelectConversation(ConversationId(1)));
    let model = cx.new(|_| Model::new(state));
    let m2 = model.clone();
    let (view, vcx) =
        cx.add_window_view(move |window, cx| TranscriptView::new(m2.clone(), window, cx));
    let h = Harness { model, view };
    open(&h.model, 1, items(0..n), false, vcx);
    frame(&h, vcx);
    (h, vcx)
}

fn anchor(h: &Harness, conv: u64, cx: &mut VisualTestContext) -> (usize, Pixels) {
    cx.update(|_, cx| h.view.read(cx).scroll_anchor(ConversationId(conv)).unwrap())
}

fn list_of(h: &Harness, conv: u64, cx: &mut VisualTestContext) -> ListState {
    cx.update(|_, cx| h.view.read(cx).list_state(ConversationId(conv)).unwrap())
}

fn stats(h: &Harness, cx: &mut VisualTestContext) -> TranscriptStats {
    cx.update(|_, cx| h.view.read(cx).stats(cx))
}

fn append(h: &Harness, text: &str, cx: &mut VisualTestContext) {
    // Drive the model the way the controller would: submit, accept, stream a token.
    cx.update(|_, cx| {
        h.model.update(cx, |m, cx| {
            if matches!(m.state.current().unwrap().run, RunState::Idle) {
                m.dispatch(Command::EditDraft("go".into()), cx);
                m.dispatch(Command::Submit, cx);
                let c = m.state.current().unwrap();
                let ev = BackendEvent {
                    conversation: c.id,
                    generation: c.generation,
                    op: c.run.op(),
                    kind: EventKind::Accepted,
                };
                m.apply_event(ev, cx);
            }
            let c = m.state.current().unwrap();
            let ev = BackendEvent {
                conversation: c.id,
                generation: c.generation,
                op: c.run.op(),
                kind: EventKind::Token(text.into()),
            };
            m.apply_event(ev, cx);
        });
    });
}

#[gpui::test]
fn mounted_rows_are_bounded_for_ten_thousand_items(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 10_000);
    frame(&h, cx);
    let s = stats(&h, cx);
    assert!(s.mounted_rows > 0, "nothing rendered");
    assert!(s.mounted_rows < 150, "mounted {} rows", s.mounted_rows);
    assert!(s.cached_blocks < 300, "cached {}", s.cached_blocks);
    assert!(s.following);
}

#[gpui::test]
fn tail_follow_pauses_when_reading_above_and_resumes_on_jump(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 200);
    assert!(stats(&h, cx).following);
    let list = list_of(&h, 1, cx);
    list.scroll_by(px(-3000.));
    frame(&h, cx);
    assert!(!stats(&h, cx).following);
    let before = anchor(&h, 1, cx);
    for i in 0..5 {
        append(&h, &format!("token {i} "), cx);
        frame(&h, cx);
    }
    // Streaming below must not move the reading position.
    assert_eq!(anchor(&h, 1, cx), before);
    assert!(!stats(&h, cx).following);
    cx.update(|window, cx| h.view.update(cx, |v, cx| v.jump_to_latest(window, cx)));
    frame(&h, cx);
    assert!(stats(&h, cx).following);
    append(&h, "more ", cx);
    frame(&h, cx);
    assert!(stats(&h, cx).following);
}

#[gpui::test]
fn prepending_history_keeps_the_viewport_anchored(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 0);
    open(&h.model, 1, items(100..300), true, cx);
    frame(&h, cx);
    let list = list_of(&h, 1, cx);
    list.scroll_by(px(-900.));
    frame(&h, cx);
    let (ix, off) = anchor(&h, 1, cx);
    assert!(ix > 3, "scrolled too little: {ix}");
    let generation = cx.update(|_, cx| {
        h.model
            .read(cx)
            .state
            .conversation(ConversationId(1))
            .unwrap()
            .generation
    });
    cx.update(|_, cx| {
        h.model.update(cx, |m, cx| {
            m.apply_event(
                BackendEvent {
                    conversation: ConversationId(1),
                    generation,
                    op: None,
                    kind: EventKind::OlderPage {
                        items: items(50..100),
                        has_older: false,
                    },
                },
                cx,
            )
        })
    });
    frame(&h, cx);
    assert_eq!(anchor(&h, 1, cx), (ix + 50, off));
}

#[gpui::test]
fn each_conversation_restores_its_own_scroll_position(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 300);
    list_of(&h, 1, cx).scroll_by(px(-1200.));
    frame(&h, cx);
    let a1 = anchor(&h, 1, cx);
    cx.update(|_, cx| {
        h.model.update(cx, |m, cx| {
            m.dispatch(Command::SelectConversation(ConversationId(2)), cx)
        })
    });
    open(&h.model, 2, items(0..50), false, cx);
    frame(&h, cx);
    assert!(
        stats(&h, cx).following,
        "fresh conversation starts at the tail"
    );
    cx.update(|_, cx| {
        h.model.update(cx, |m, cx| {
            m.dispatch(Command::SelectConversation(ConversationId(1)), cx)
        })
    });
    frame(&h, cx);
    assert_eq!(anchor(&h, 1, cx), a1);
    assert!(!stats(&h, cx).following);
}

#[gpui::test]
fn expanding_a_tool_does_not_move_the_anchor(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 300);
    list_of(&h, 1, cx).scroll_by(px(-1500.));
    frame(&h, cx);
    let (ix, off) = anchor(&h, 1, cx);
    // First tool row at or below the top item.
    let tool = (ix..300).find(|i| i % 5 == 3 && *i >= ix).unwrap() as u64;
    cx.update(|_, cx| {
        h.view.update(cx, |v, cx| {
            v.toggle_tool(ConversationId(1), ItemId(tool), cx)
        })
    });
    frame(&h, cx);
    assert!(cx.update(|_, cx| h.view.read(cx).is_expanded(ConversationId(1), ItemId(tool))));
    assert_eq!(anchor(&h, 1, cx), (ix, off));
    cx.update(|_, cx| {
        h.view.update(cx, |v, cx| {
            v.toggle_tool(ConversationId(1), ItemId(tool), cx)
        })
    });
    frame(&h, cx);
    assert_eq!(anchor(&h, 1, cx), (ix, off));
}

#[gpui::test]
fn selection_copies_across_rows_including_offscreen(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 10_000);
    let conv = ConversationId(1);
    let sel = Selection {
        anchor: DocPos {
            item: ItemId(10),
            block: 0,
            offset: 7,
        },
        head: DocPos {
            item: ItemId(12),
            block: 1,
            offset: 4,
        },
    };
    cx.update(|_, cx| {
        h.view
            .update(cx, |v, cx| v.set_selection(conv, Some(sel), cx))
    });
    frame(&h, cx);
    let text = cx
        .update(|_, cx| h.view.read(cx).selection_text(cx))
        .unwrap();
    assert!(
        text.starts_with("number 10 please look at src/lib.rs"),
        "{text}"
    );
    assert!(text.contains("Paragraph 11 with bold and code."));
    assert!(text.contains("fn f11()"));
    assert!(text.ends_with("A fa"), "{text:?}");
    assert!(stats(&h, cx).selection_len > 0);
    // Streaming appends do not corrupt the selection.
    let before = text.clone();
    append(&h, "streamed ", cx);
    frame(&h, cx);
    assert_eq!(
        cx.update(|_, cx| h.view.read(cx).selection_text(cx))
            .unwrap(),
        before
    );
}

#[gpui::test]
fn mouse_drag_selects_across_rows_and_select_all_copies_everything(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 40);
    frame(&h, cx);
    let hits: Vec<_> = cx.update(|_, cx| h.view.read(cx).registry.borrow().blocks.clone());
    assert!(
        hits.len() > 3,
        "expected several painted blocks, got {}",
        hits.len()
    );
    let visible: Vec<_> = hits
        .iter()
        .filter(|h| h.bounds.top() >= px(0.) && h.bounds.bottom() <= px(1000.))
        .collect();
    assert!(
        visible.len() > 3,
        "expected visible blocks, got {}",
        visible.len()
    );
    let a = visible.first().unwrap().bounds;
    let b = visible.last().unwrap().bounds;
    cx.simulate_mouse_down(a.center(), gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(b.center(), gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_up(b.center(), gpui::MouseButton::Left, gpui::Modifiers::none());
    let text = cx
        .update(|_, cx| h.view.read(cx).selection_text(cx))
        .unwrap_or_default();
    assert!(
        text.contains("\n\n"),
        "drag should span multiple blocks: {text:?}"
    );

    cx.update(|_, cx| h.view.update(cx, |v, cx| v.select_all(cx)));
    let all = cx
        .update(|_, cx| h.view.read(cx).selection_text(cx))
        .unwrap();
    assert!(all.starts_with("prompt number 0") && all.contains("Short reply 39."));
    cx.update(|_, cx| h.view.update(cx, |v, cx| v.clear_selection(cx)));
    assert!(!cx.update(|_, cx| h.view.read(cx).has_selection()));
}

#[gpui::test]
fn load_older_is_requested_once_near_the_top(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 0);
    open(&h.model, 1, items(100..160), true, cx);
    frame(&h, cx);
    list_of(&h, 1, cx).scroll_to(ListOffset {
        item_ix: 0,
        offset_in_item: px(0.),
    });
    frame(&h, cx);
    cx.run_until_parked();
    let loading = cx.update(|_, cx| {
        h.model
            .read(cx)
            .state
            .conversation(ConversationId(1))
            .unwrap()
            .loading_older
    });
    assert!(loading, "reaching the top should request the previous page");
}

#[gpui::test]
fn empty_and_unselected_states_render(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 0);
    frame(&h, cx);
    assert_eq!(stats(&h, cx).mounted_rows, 0);
}

#[gpui::test]
fn keyboard_select_all_copy_extend_and_escape(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 30);
    let conv = ConversationId(1);
    let handle = cx.update(|_, cx| h.view.read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&handle, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-a");
    assert!(cx.update(|_, cx| h.view.read(cx).has_selection()));
    cx.simulate_keystrokes("ctrl-c");
    let copied = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
    assert!(copied.starts_with("prompt number 0") && copied.contains("Short reply 29."));
    cx.simulate_keystrokes("escape");
    assert!(!cx.update(|_, cx| h.view.read(cx).has_selection()));
    // shift-right extends an existing caret by whole graphemes.
    let sel = Selection::caret(DocPos {
        item: ItemId(0),
        block: 0,
        offset: 0,
    });
    cx.update(|_, cx| {
        h.view
            .update(cx, |v, cx| v.set_selection(conv, Some(sel), cx))
    });
    cx.simulate_keystrokes("shift-right shift-right shift-right");
    assert_eq!(
        cx.update(|_, cx| h.view.read(cx).selection_text(cx))
            .unwrap(),
        "pro"
    );
    cx.simulate_keystrokes("ctrl-shift-end");
    let tail = cx
        .update(|_, cx| h.view.read(cx).selection_text(cx))
        .unwrap();
    assert!(tail.ends_with("Short reply 29."));
}

#[gpui::test]
fn stress_content_renders_without_panicking(cx: &mut TestAppContext) {
    let (h, cx) = setup(cx, 0);
    let long = "x".repeat(5000);
    let nasty = vec![
        TranscriptItem {
            id: ItemId(1),
            at: 0,
            kind: ItemKind::User {
                text: "مرحبا بالعالم שלום עולם 👨‍👩‍👧‍👦 e\u{301}\u{301}".into(),
                attachments: vec![Attachment {
                    path: "/missing".into(),
                    name: "missing.txt".into(),
                    size: None,
                    error: Some("File not found".into()),
                }],
                delivery: Delivery::Unknown,
                steer: false,
            },
        },
        TranscriptItem {
            id: ItemId(2),
            at: 0,
            kind: ItemKind::Assistant {
                text: format!(
                    "**unclosed and `tick\n\n```rust\nfn open() {{\n\n- a\n    - b\n  - c\n\n/very/long/{long}/path.rs {long}"
                ),
                streaming: true,
            },
        },
        TranscriptItem {
            id: ItemId(3),
            at: 0,
            kind: ItemKind::Tool(ToolCall {
                call_ref: None,
                call_id: None,
                name: "bash".into(),
                input: long.clone(),
                output: long,
                truncated: true,
                full_len: 2_000_000,
                status: ToolStatus::Failed,
            }),
        },
        TranscriptItem {
            id: ItemId(4),
            at: 0,
            kind: ItemKind::Assistant {
                text: String::new(),
                streaming: true,
            },
        },
        TranscriptItem {
            id: ItemId(5),
            at: 0,
            kind: ItemKind::Notice {
                text: "Provider unavailable".into(),
                level: NoticeLevel::Error,
            },
        },
    ];
    open(&h.model, 1, nasty, false, cx);
    cx.update(|_, cx| {
        h.view
            .update(cx, |v, cx| v.toggle_tool(ConversationId(1), ItemId(3), cx))
    });
    cx.update(|_, cx| h.view.update(cx, |v, cx| v.select_all(cx)));
    frame(&h, cx);
    assert!(stats(&h, cx).mounted_rows >= 1);
    let all = cx
        .update(|_, cx| h.view.read(cx).selection_text(cx))
        .unwrap();
    assert!(all.contains("שלום") && all.contains("fn open()"));
}

/// Resident memory of this process, in kilobytes.
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.split_whitespace().next()?.parse().ok())
        })
        .unwrap_or(0)
}

/// Walk the whole of a 10,000-message conversation from top to bottom, again and again, with
/// the model streaming into it while it is being read: what is built per frame, what is cached
/// and the memory of the process all stay bounded, so a long session does not grow.
#[gpui::test]
fn repeated_traversal_of_ten_thousand_items_with_live_updates_stays_bounded(
    cx: &mut TestAppContext,
) {
    let (h, cx) = setup(cx, 10_000);
    frame(&h, cx);
    let list = list_of(&h, 1, cx);
    let mut max_mounted = 0;
    let mut max_cached = 0;
    let mut rss: Vec<u64> = Vec::new();
    for pass in 0..6 {
        for (step, ix) in (0..10_000usize).step_by(89).enumerate() {
            list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
            frame(&h, cx);
            // Live output arrives while the person reads somewhere else.
            if step % 20 == 0 {
                append(&h, "streaming ", cx);
                frame(&h, cx);
            }
            let s = stats(&h, cx);
            assert!(
                s.mounted_rows > 0,
                "pass {pass}, item {ix}: nothing rendered"
            );
            max_mounted = max_mounted.max(s.mounted_rows);
            max_cached = max_cached.max(s.cached_blocks);
        }
        rss.push(rss_kb());
    }
    eprintln!(
        "traversal: max mounted rows {max_mounted}, max cached blocks {max_cached}, RSS per pass (KB) {rss:?}"
    );
    assert!(max_mounted < 150, "mounted {max_mounted} rows at once");
    assert!(max_cached <= 2000, "cached {max_cached} blocks");
    // The first passes fill the caches; after that nothing may keep growing.
    let growth = rss[5].saturating_sub(rss[1]);
    assert!(
        growth < 40 * 1024,
        "memory grew {growth} KB over four more traversals: {rss:?}"
    );
}
