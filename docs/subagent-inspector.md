# Subagent inspector

Subagents and workspace Changes are separate inspector tabs. The conversation header and
command palette provide a direct **Show subagent tasks** entry point, including when the
inspector is closed or uses a narrow-window drawer. Opening activity never moves the
composer away from the main conversation.

## Presentation contract

- A flat task directory reflects the actual root-session → child relationship. There is
  no invented nested tree, percentage, ETA, phase, or individual cancellation control.
- Titles are UI-derived from a short first task sentence (including an initial `You are`
  instruction), with a stable `Subagent <id>` fallback. They are not engine-authored agent
  names. The engine currently provides only a bounded task excerpt, not a complete prompt.
- Task rows wrap their descriptions. Running, Pending, Waiting, Completing, Completed,
  Failed and Stopped have explicit text and library icons; unknown statuses remain Unknown.
- Counts cover the whole available directory. Active tasks come first, then stopped/failed
  work, then completed work, preserving engine order within each group. At most 100 task
  rows mount, with an explicit total/bound notice. This is not pagination.
- Selecting a task replaces the directory with its focused read-only activity view.
  **All tasks** returns to the directory. Task execution status is independent of activity
  transport: Loading, Live, Snapshot, and unavailable/disconnected are not conflated.
- The activity view retains the adapter's bounded plain-text previews and shortening
  notices. It is not a full-history browser or a Markdown editor. Changes remain one tab
  away and retain the selected file/diff.
- Labels, excerpts, sorting and status counts are prepared on model updates, not by prompt
  parsing during render. Child content, subscription settlement, selection and stale-event
  rejection remain core/backend-owned.
- Palette entries **Inspect subagent <id>: …** and **Show all subagent tasks** provide
  non-pointer navigation. They exist only for an advertised directory; existing work is
  inspectable when new calls are disabled.

## Reproducible, credential-free fixture

```sh
cargo run -p pipkin-app --release -- --demo subagents --data-dir /tmp/pipkin-subagents-demo
```

Use a new disposable directory. This scenario explicitly displays **Demo · simulated
agent**, with frozen running/completed/failed counters. Its child snapshots contain
additional simulated-content notices. It executes no counter, tool or provider request
and proves nothing about a real engine's progress. Demo child selection is allowed only
for an opened session explicitly advertising that synthetic service; it does not enable
subagents in unsupported demo sessions or change real-mode preferences policy.

## Verification — 2026-10-08

**Automated:** workspace tests, Clippy for all targets, formatting, and release build passed.
Presentation tests cover Unicode/long excerpts, engine statuses and shipped icon paths,
complete counts with a bounded directory, selected detail outside the directory bound,
conversation replacement, and independent loading/live/snapshot/disconnected labels.
Palette tests cover advertised versus unavailable directories and reading existing work
with new calls disabled. Demo/backend/core tests cover matching synthetic snapshots and
rejection of wrong child/conversation/generation events without modifying parent drafts,
transcript, run state, read-only policy or demo mode.

**Native measured:** launched only an owned throwaway-profile demo window on the current
Wayland workspace. Every synthetic key/text input used the PID-pinned `scripts/guard.sh`.
Captured dark desktop (1440×960), light desktop with Large text, and light Large-text
narrow-window drawer (720×960). After one fix batch, captured completed/running focused
snapshots and the deduplicated directory. Keyboard palette navigation selected Counter B,
returned to all tasks, and selected Counter A. The owned window was closed afterward.
OCR extracted titles, all three status labels, tabs, All tasks, Snapshot, child contents,
and the read-only/main-conversation routing footer. The initial owned-window AT-SPI tree
exposed task buttons with status/action labels and separate inspector landmarks.

**Limits / unverified:** the tool harness cannot open PNGs for visual inspection; captures
and OCR are not an aesthetic or contrast acceptance review. Screen-reader interaction,
owner acceptance, real-engine live/disconnection interaction, and native macOS interaction
remain unverified. The later 1010px capture still used the existing open drawer and is not
proof of a 280px docked inspector. Long-content/many-child behavior has pure tests, not a
complete native visual matrix. No owner profile, credentials, clipboard, compositor config
or unrelated window was changed. Local captures/logs are in
`/tmp/pipkin-subagents-visual.UjBulA/` (temporary, not tracked release artifacts).
