# Verification and the scorecard

Plan-derived rule: *if an IME, screen reader, scale configuration or measurement tool is unavailable, record the gate as unverified rather than passed.* Keep four buckets: measured, observed, failed, unverified. Label what a number is (CPU frame submission ≠ display presentation).

## What this prototype actually established (Intel RPL-S via Vulkan, Hyprland 0.56.2, 2560×1080@60, scale 1)
- Automated: 135 tests (core 13, ui 73, app 49), clippy and fmt clean, kill-test for storage.
- Native: workflow end to end; mouse cross-block selection + copy/paste; steer/queue/cancel (stays Stopping until confirmed, queue kept); wheel scroll-away during streaming stays put with "Jump to latest"; 1440/1024/720 layouts; light/dark; `kill -9` draft recovery; AT-SPI tree (landmarks, log, articles, status).
- Measured: input-to-frame p50 3.3 ms / p95 4.8 ms (n=628); transcript CPU frame p50 1.1 ms / p95 1.7 ms (600 frames); window mapped 175–218 ms warm, ~200–260 ms with binary pages evicted; idle RSS 140 MB, 148 MB after heavy typing in the 10k conversation; mounted rows 4–6 for 10,000 items.
- **Unverified (and why):** real IME (fcitx5 present, no composing engine installed), screen reader (Orca absent), 125%/150% scale (would change the user's display), minimize/restore & suspend/resume, true cold boot (needs root to drop caches), display-presentation latency, repeated-traversal memory. Recommendation was **conditional go**.

## Test patterns that paid off
- Pure-model tests for editor/selection/markdown; GPUI tests (`TestAppContext`) for input-handler behavior (Enter vs Shift+Enter vs Enter-while-marked, IME commit/cancel, set_text doesn't emit Changed, growth to 8 lines), list anchoring (prepend keeps `(item_ix+n, offset)`), tail pause/resume, per-conversation scroll restore, tool expansion not moving the anchor, 10,000 items with bounded mounted rows, drag selection across rows.
- Headless scenario tests: `AppState` + `DemoBackend` at speed 0 through every scenario; determinism by comparing `Debug` of event sequences across identical seeds.
- GPUI's test text system is a no-op, so layout metrics in tests are not real glyph metrics; confirm wrapping and long-token behavior natively.
- Rules for tests: use `cx.background_executor().timer`, not smol timers.

## Pitfalls in claiming results
- Don't call a timer around a state update "input-to-paint".
- Opacity/translucency in captures is the compositor, not necessarily your app.
- Keep a visible "Demo · simulated agent" marker and "Workspace changes · Demo" title; demo content must never imply real execution.
