# 10-minute owner review

Build and launch (release, Wayland):

```sh
cargo run -p desktop-app --release -- --demo normal
```

Use a fresh data directory for a clean first run: add `--data-dir /tmp/pi-review`.

1. **Orient (1 min).** Dark theme, 1440×960. Navigation (projects, conversations, search), conversation, and the changes inspector. The "Demo · simulated agent" chip is visible.
2. **Send a prompt (1 min).** Pick "Fix failing test…", type a prompt, press Enter. Follow tokens, tool rows (read, edit, a failing test run, a correction), and the three changed files.
3. **Scroll away during streaming (1 min).** Scroll up while it streams. The view must not jump. "Jump to latest" appears; click it to resume following.
4. **Select and copy across messages (1 min).** Drag from one message through a code block into another, including past the viewport edge. Ctrl+C, paste into the composer to check. Try Ctrl+A in the transcript.
5. **Expand a tool and open a diff (1 min).** Click a tool row to expand it. Click a file in the inspector or a file reference in the summary: the diff appears, the transcript stays where it was.
6. **Steer, queue, cancel (1.5 min).** Switch scenario with Ctrl+K → "Demo scenario: followup". Send a prompt. While it runs: send a steer (Enter), queue two prompts (Ctrl+Enter), remove one from the queue, press Stop and note that it stays "Stopping…" until confirmed.
7. **Switch conversations with unsent text (1 min).** Type text without sending, switch conversations (navigation or Alt+↓), switch back: the draft and scroll position are restored.
8. **Relaunch and recover (1 min).** Wait for "Draft saved", quit (Ctrl+Q), relaunch with the same `--data-dir`: the draft, selected conversation, theme, and pane widths return.
9. **Narrow window and alternate theme (1.5 min).** Tile the window to ~720 px wide, switch to Light (Ctrl+,), and repeat steps 2–3. Navigation and inspector are temporary, mutually exclusive panels; Escape closes them and focus returns to where it was.
10. **Failure cases (bonus).** Scenarios `failure`, `unknown`, `stressed`, `large`, and "Inject draft save failure" are in the palette under Developer.
