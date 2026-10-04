# Virtualized streaming transcript and selection

## List mechanics (G/elements/list.rs, uniform_list.rs)
- `list(ListState, |ix, window, cx| ..)` caches item heights. Items outside the viewport must not change height unless you `splice`/`reset`/`remeasure_items`. Visible items re-render every frame; overdraw items render only if unmeasured. A width change invalidates **all** cached heights.
- `ListState::new(count, ListAlignment::Bottom|Top, overdraw_px)` (Zed's agent thread: Top, 2048 px overdraw, Tail follow). `measure_all()` is expensive on first frame. `reset(n)` drops scroll state.
- Streaming token: `remeasure_items(ix..ix+1)` (absolute-pixel anchor). New item: `splice(len..len, 1)`. Removal: `splice(range, 0)`. `remeasure()` (all items) keeps a proportional position, for font-size changes.
- Follow: `set_follow_mode(FollowMode::Tail)`; scrolling up (distance < 0) stops following; returning to within 1px of the bottom resumes. `pause_following_tail()` freezes without leaving Tail. `scroll_to_end()` anchors at `item_ix = count` so growth of the last item stays pinned. On completion, if `is_following_tail()` call `scroll_to_end()` again so the final height is followed.
- **Prepend history without a jump:** `let top = list.logical_scroll_top(); list.splice(0..0, n); list.scroll_to(ListOffset{ item_ix: top.item_ix + n, offset_in_item: top.offset_in_item })`. Verified by tests (anchor stays at item_ix+n, same offset).
- Reentrancy: `set_scroll_handler` runs while the list's `RefCell` is borrowed; read `logical_scroll_top()` there and you panic. Defer with `cx.defer`.
- `uniform_list(id, count, |range, w, cx| Vec<Row>)` measures one item and lays the rest out linearly; `UniformListScrollHandle::scroll_to_item`; use for the diff (virtualized 2,000+ lines).

## Choices that worked
- **Index 0 header row** (fixed 40 px: "Loading earlier…" / "Beginning of conversation") so loading a page never shifts the viewport. Residual: if the header is the scroll-top item when a page arrives, the view can shift up to 40 px.
- **One `ListState`, expanded-tool set, selection and parsed document per conversation**, in a bounded map (24). Switching away and back restores scroll anchor, expansion and selection.
- Stable identity: list index == transcript item index; per-item UI state (expanded tools) is keyed by `ItemId` in the view, never in the render closure (Zed keeps toggles in `HashSet`s in `EntryViewState`).
- Request `LoadOlder` once near the top (guard with `loading_older`).
- Mounted rows stayed bounded (4–6) on a 10,000-item conversation; frame CPU p95 ≈ 1.7 ms over 600 frames.

## Markdown pipeline
`pulldown-cmark` → block model (paragraph, heading, list, quote, fenced code, inline emphasis/strong/code/links) with plain selectable text and styled runs. Malformed/unterminated Markdown during streaming must not panic (an incomplete fence renders as code). Cache parsed blocks by `(ItemId, content revision)`, LRU-bounded (2,000); only the streaming tail reparses per token batch. Zed's approach for reference (M = `zed/crates/markdown/src`): `append` copies the source string (O(n)), `parse` coalesces to one in-flight `background_spawn` + `should_reparse`, full pulldown reparse each time (not incremental), results swapped in on the foreground, **old content stays visible** until the new parse lands (`reset`, markdown.rs:1037), code-block highlights computed on the background thread and reused by `Arc::ptr_eq` for unchanged blocks while streaming (test `test_code_block_highlights_reused_when_streaming`).
Highlighting here is a small hand-written tokenizer (rust, js/ts, python, json, bash, toml/yaml, diff, plain fallback) mapped to theme `syn_*` tokens; fine for a demo, replace with tree-sitter/syntect for production.

## Document-level selection (`TranscriptDocument`)
- Position = (ItemId, block index, byte offset); `Selection { anchor, head }` independent of mounted rows. `selected_text(items)` walks the model, so copy includes offscreen rows, code blocks and tool rows; clamp offsets when streaming reshapes a block.
- Each frame, every mounted text block registers its layout (bounds + `WrappedLine`s) in a hit-test registry that is **cleared and repopulated per frame**, which is what makes recycled rows safe. Mouse down/drag → find block under pointer (clamp to first/last visible when outside), then `closest_index_for_position`. Auto-scroll while dragging beyond the edges.
- Keyboard (context `Transcript`): ctrl-a, ctrl-c, shift-left/right/up/down, ctrl-shift-home/end, Escape (only when focused). Double-click word, triple-click paragraph.
- Zed's markdown selection (M/markdown.rs:1424, selection.rs:104) uses byte offsets into the source mapped through `source_mappings`, with modes Character/Word/Line/All, and on copy rebalances half-cut delimiters (`**bold*|*`) so copied markdown is well-formed. Consider that for "copy as markdown".
- Native check: a real mouse drag from prose through a list into a code block, Ctrl+C, paste into the composer reproduced all selected text.

## Design decisions in the rows
User prompt is the only card; assistant is flowing markdown (author label "Pi"); tool rows are compact and expandable with status text+icon; bounded output preview with "Output truncated (N KB total)"; code blocks **wrap** (not horizontal scroll) so hit-testing is exact; static (non-animated) streaming indicator; text column bounded to ~760 px × text scale. Inline file references and tool-input paths matching a change dispatch `SelectChange` without touching scroll or draft.
