# Dependency decisions

- GPUI and gpui_platform are pinned to Zed `a84689073d296dfd39987bc7dd478e43ef76d83a` (gpui 0.2.2), features `wayland` and `font-kit`, default features off (no X11).
- The independent workspace excludes `zed/`, which stays a read-only reference.
- Toolchain: Zed's file asks for 1.98.1; the installed 1.99.0 builds the pinned revision cleanly, so 1.99.0 is pinned.
- Zed's root `[patch.crates-io]` entries (calloop, async-task, async-process, ...) are not applied. The build and window launch work without them. Revisit if event-loop or async behaviour misbehaves.
- Licenses: gpui and gpui_platform are Apache-2.0. Zed `ui`, `markdown`, `editor`, `agent_ui` are GPL-3.0-or-later and are used only as references. No adapted source or assets so far.
