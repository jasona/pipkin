# Composer / text input (ComposerEditor + EditorModel)

## Split
`EditorModel` is pure Rust (bytes, grapheme boundaries, UTF-16 conversion, selection with reversal, marked range, undo/redo with typing coalescing and IME-commit collapse, word/line motion, CRLF normalization). It has 20 unit tests: ZWJ families, flags, combining marks, UTF-16 round trips, undo through emoji edits, marked-text commit/cancel. `ComposerEditor` is the GPUI entity + custom `Element`. Start from `zed/crates/gpui/examples/input.rs` but note it is single-line, returns no IME bounds, and ignores most editing behavior.

## Platform contract (G/input.rs, G/platform.rs:2187-2336)
- `EntityInputHandler`: all ranges are **UTF-16**. Implement `text_for_range` (fill `actual_range`), `selected_text_range`, `marked_text_range`, `unmark_text`, `replace_text_in_range`, `replace_and_mark_text_in_range`, `bounds_for_range` (real caret rects so the candidate popup sits right), `character_index_for_point`.
- Register in **paint**: `window.handle_input(&focus_handle, ElementInputHandler::new(bounds, entity), cx)`; it is active only when focused and only for the next frame.
- `accepts_text_input` drives enabling IME each frame on Wayland (`zwp_text_input_v3`). A **disabled** composer must return false and be read-only.
- **Call `window.invalidate_character_coordinates()` whenever the caret moves** or the IME popup stays at the old place (Zed does: editor/selection.rs:1543). On Wayland `update_ime_position` is skipped while pre-edit text exists. This prototype does not call it yet.
- Enter during marked text must go to the IME. Only emit `Submit` when `marked_range` is none. Shift+Enter inserts a newline. Tests simulate composition by calling `replace_and_mark_text_in_range` then Enter.
- Zed editor details worth copying: clip offsets to char boundaries (`clip_offset_utf16`) and report `adjusted_range`; during composition treat an empty replacement range as "insert at cursor"; disable autoclose/auto-surround while composing; `selected_text_range` returns None when input disabled (prevents the IME menu while a key is held); underline marked text with a 1px highlight.

## Layout and painting
- `shape_line` is single-line only (debug-asserts no `\n`). Use `shape_text(text, size, runs, wrap_width, None)` which returns one `WrappedLine` per `\n`-separated line; `position_for_index` / `closest_index_for_position` work per line relative to its origin. Accumulate `line.size(line_height).height` for tops.
- `TextRun`s must cover the whole string or GPUI logs a warning. Runs cannot change font size (so inline code can't be smaller than prose in one paragraph).
- GPUI's `LineLayoutCache` is two-frame (reuse per frame, not LRU); changing wrap width forces re-wrap; cached views carry layouts forward.
- Height: use `request_measured_layout` so the composer grows with wrapped content up to 8 lines, then scrolls internally keeping the caret visible; pass the wheel through when it cannot scroll.
- Home/End by visual wrapped row; Up/Down use wrapped-layout positions with a remembered goal column.
- Blink: stop under `reduced_motion`; otherwise blink with a timer that stops after idle. Never animate the whole window.

## API as built
`ComposerEditor::new(window, cx)` / `single_line(window, cx)` (palette, rename, search: no wrap, newlines→spaces, emits `Up/Down`), `text()`, `set_text(&str, cx)` (resets undo, does **not** emit Changed), `set_placeholder`, `set_label(label, cx)` (a11y), `set_disabled(bool, cx)`, `select_all(cx)`, `focus_handle(cx)`. Events: `Changed` (every user edit incl. IME), `Submit`, `Escape`, `Up`, `Down`.

## Measuring
`text::latency::{mark, painted, stats}` records handler-call → composer-paint-end. Measured 628 keystrokes at 30 ms cadence: p50 3.3 ms, p95 4.8 ms (also in a 10,000-message conversation). It is CPU frame submission, not display presentation and excludes compositor delivery; say so wherever you quote it.
