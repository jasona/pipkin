# Pipkin (GPUI prototype): agent instructions

Native Rust/GPUI desktop app for Omarchy/Hyprland/Wayland with a simulated Pi backend. Plan: `docs/gpui-prototype-plan.md`. Status and gates: `docs/scorecard.md`. Run: `cargo run -p pipkin-app --release -- --demo normal`.

## Read the skill first
Before touching GPUI views, the composer/text input (IME), the transcript/list/selection code, persistence, the state core, or native testing, read the **`gpui-desktop-app` skill**:
- Claude Code: `.claude/skills/gpui-desktop-app/SKILL.md`
- Codex / others: `.agents/skills/gpui-desktop-app/SKILL.md` (identical copy)
Start with its "12 rules", then open only the reference file matching your task (`references/`).

## Non-negotiables
- Keep `pipkin-core` free of GPUI. Every backend event carries `(conversation, generation, op)` and is stale-guarded. Render does no I/O or parsing.
- `zed/` is a read-only reference checkout. Never edit or build inside it. Zed's GPL crates (`ui`, `markdown`, `editor`, `agent_ui`) are read, not copied.
- Before finishing: `cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --all --check` must pass.
- **Native testing safety:** send synthetic input only through `scripts/guard.sh` (it refuses unless Pipkin is the active window). Do not move, resize, focus, or switch workspaces for windows you did not launch. Stay on the current workspace. Do not edit the user's Hyprland/Omarchy config. Do not install packages or change display scale without explicit approval.
- Scorecards separate measured / observed / failed / unverified. Never mark IME, screen reader, display scale, suspend/resume, cold boot, or display-presentation latency as passed without the real thing.
- Keep the "Demo · simulated agent" marker; demo content must never imply real execution.

## If you change the skill
Edit `.claude/skills/gpui-desktop-app/` and re-copy it to `.agents/skills/gpui-desktop-app/` so both stay identical.
