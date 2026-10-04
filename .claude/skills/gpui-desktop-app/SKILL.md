---
name: gpui-desktop-app
description: Build, debug, test and verify native desktop apps with GPUI (Zed's UI framework) outside the Zed repo, especially chat/agent style apps on Linux/Wayland (Hyprland/Omarchy). Use for GPUI views and entities, a custom multiline text editor with IME, a virtualized streaming transcript with cross-message selection, a pure-Rust state core with a simulated backend, SQLite draft persistence, GPUI tests, Wayland/Hyprland native testing, and honest verification scorecards. Distilled from the Pi Desktop prototype in this repo plus a read of Zed's source at rev a846890.
---

# GPUI desktop app: lessons from the Pi Desktop prototype

Everything here was learned building `crates/desktop-core`, `desktop-ui`, `desktop-app` in this repo (135 tests, running native Wayland app) and by reading Zed's source at the pinned rev. File references under `zed/` are to that checkout. Read the reference file that matches your task; do not read them all.

## Start here: the 12 rules that cost the most time

1. **Pure core, thin UI.** Put identities, run-state transitions, queues, availability and draft/save state in a framework-free crate with tests. The UI sends `Command`s, the core returns `Effect`s and `Note`s. This made 13 core tests cover the hard correctness rules without GPUI. → `references/architecture.md`
2. **Guard every async event** with `(conversation, generation, op)`. Route by conversation id, never by "what is selected". Drop stale generations and non-live ops. → `references/architecture.md`
3. **Render does no I/O and no parsing.** Parse markdown, highlight, hit SQLite and shape expensive text off render (cache by item id + revision). Zed parses on a background task, coalesces appends to one in-flight parse plus one follow-up, and keeps old content visible until the new parse lands. → `references/zed-practices.md`
4. **Lists: `list` for variable height, `uniform_list` for fixed height.** Streaming uses `remeasure_items`; structural changes use `splice`; prepend = capture `logical_scroll_top`, `splice(0..0, n)`, `scroll_to(item_ix + n, same offset)`. `FollowMode::Tail` follows only while at bottom. → `references/transcript-and-lists.md`
5. **Selection belongs to a document model, not to painted rows.** Addresses are (item id, block, byte offset); mounted rows only hit-test and paint. Copy reads the model so offscreen rows work. → `references/transcript-and-lists.md`
6. **Text input is the main risk.** Implement `EntityInputHandler` with UTF-16 ranges, marked text, real `bounds_for_range`; Enter must never submit while composing; call `window.invalidate_character_coordinates()` when the caret moves (this prototype does NOT yet; see open items). → `references/text-and-ime.md`
7. **Tasks and subscriptions die when dropped.** Store `Task`s you need, `.detach()` fire-and-forget, keep `Subscription`s in `_subs: Vec<Subscription>`. Use the inner `cx` inside `update` closures; nested updates of the same entity panic. → `references/gpui-gotchas.md`
8. **`role()`/aria need an element id** (`div().id(..).role(..)`), and ids must be unique per frame. Verify the real tree with AT-SPI (`python3` + `gi.repository.Atspi`), not by assuming. → `references/gpui-gotchas.md`
9. **Pin GPUI to one full Zed revision** in an independent workspace, enable `wayland` explicitly, and read `references/build-and-deps.md` about the **calloop fork Zed patches in** (not applied in this prototype yet).
10. **Native testing can hit the user's other windows.** Focus can be stolen mid-run. Guard every synthetic input (`scripts/guard.sh`), never move/resize/focus windows you didn't launch, stay on the current workspace, and do not touch the user's compositor config. → `references/native-testing.md`
11. **Scorecards must separate measured / observed / failed / unverified.** Never mark IME, screen reader, display scale, suspend/resume or cold boot "passed" without the real thing. → `references/verification.md`
12. **Parallel workers need owned directories** and must keep the shared crate compiling; a broken file in one module blocks everyone's `cargo check`. → `references/process.md`

## Layout used (copy it)

```
Cargo.toml  rust-toolchain.toml        independent workspace, excludes zed/
crates/desktop-core/                   ids, model, protocol (Command/Effect/Note/BackendRequest/BackendEvent), state machine, Backend trait
crates/desktop-ui/src/{theme,assets,model}.rs   semantic tokens, bundled fonts/icons, Model entity (AppState + effect handler)
crates/desktop-ui/src/text/            EditorModel (pure) + ComposerEditor (GPUI) + latency probe
crates/desktop-ui/src/transcript/      markdown, highlight, document (selection), view
crates/desktop-ui/src/shell/           workspace, nav, center, inspector (diff), overlays, commands, controls
crates/desktop-app/                    main, controller (effects, event pump), adapters/demo.rs (+script JSON), storage.rs (SQLite)
fixtures/scenarios/*.json              versioned event scripts      assets/ fonts+icons with PROVENANCE.md
```

## Useful commands

```sh
cargo run -p desktop-app --release -- --demo normal --data-dir /tmp/pi-x
cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --all --check
RUST_LOG=warn ...   # GPUI/adapter logs; needs env_logger::init() in main
```
In-app: Ctrl+K palette → "Developer: Log performance stats" writes frame p50/p95, input-to-frame latency and RSS to the log.

## Open items this prototype did not finish (do these next)
- Call `window.invalidate_character_coordinates()` on caret moves (IME popup position) and verify with a real IME (fcitx5 engine) and Orca.
- Apply Zed's `[patch.crates-io]` calloop/async-task forks (see build-and-deps) and re-measure.
- Inline code runs use the prose font size (text runs cannot change size); Lilex looks large.
- 125%/150% scale, suspend/resume, minimize/restore, true cold boot, display-presentation latency: unverified.
- Consider `.cached(style)` on large static panels (Zed does for docks/panes only), the `profiler` feature + `dev::ToggleFpsOverlay`-style overlay, and `ZED_MEASUREMENTS=1`.
