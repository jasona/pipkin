# Platforms: what is supported, what was checked, what blocks the rest

Support is claimed only where it was run. "Checked here" means on the author's machine (Arch/Omarchy,
Hyprland 0.56, Intel + NVIDIA, 125% scale); nothing below was run on another computer.

| Target | Status | Evidence |
| --- | --- | --- |
| Omarchy/Arch, Wayland (Hyprland) | **Supported (alpha)** | Built, packaged (`makepkg`), install-checked in a scratch prefix, real-engine suite (31 tests), 1500-prompt soak, by-hand use with a real model |
| Arch/other Linux, X11 (also Xwayland) | **Builds and starts; checked under Xwayland only** | The `x11` GPUI feature is enabled alongside `wayland` (one binary; the display server picks at run time). Launched with `WAYLAND_DISPLAY` unset on Xwayland: it renders correctly. A pure X11 session, other window managers, and IME/screen reader on X11 were not tried |
| Other Wayland compositors (GNOME, KDE) | **Unverified** | Same Wayland code path as Hyprland, but portals (file picker), decorations, clipboard and IME were not tried on them |
| Other Linux distributions | **Installable, unverified** | `packaging/install.sh` installs from the release tarball into `~/.local` with upgrade and rollback (checked in a scratch prefix on this machine). It needs Node >= 22.19 and the Wayland/X11 libraries; no other distribution was tried |
| macOS | **Not supported** | See the blockers below |
| Windows | **Not supported** | See the blockers below |

## What blocks macOS and Windows (from a read of the code; no cross build was attempted)

- **Engine transport** (`crates/pi-client/src/unix.rs`): Unix-domain sockets with `SO_PEERCRED` peer
  checks and owner-only directory modes. macOS has a close equivalent (`LOCAL_PEERCRED`); Windows needs a
  named-pipe transport with an equivalent access check, which the plan requires before shipping.
- **Engine lifecycle** (`crates/pipkin-app/src/adapters/pi/engine.rs`): finds the engine's processes by reading
  `/proc` and signals them with `libc::kill` and process groups. Needs a per-OS process-ownership layer
  (job objects on Windows, `proc_pidinfo` on macOS).
- **Small `/proc` reads**: random UUIDs (`/proc/sys/kernel/random/uuid`) and RSS (`/proc/self/status`).
- **Paths and launching** (`launch.rs`, `platform.rs`): XDG directories, `xdg-terminal-exec`, terminal and
  editor discovery assume Linux conventions.
- **GPUI platform features** are `wayland`/`x11` only; macOS and Windows backends exist in GPUI but were not
  enabled or qualified.
- **Packaging, signing and updates**: none for these systems (no signed/notarized app, no signed installer).
- **Accessibility and IME** would need VoiceOver and Narrator/NVDA passes; none were done.

## Credentials and tool discovery

Pipkin stores no credentials. Pi's own mechanism (login, API keys, `models.json` under `~/.pi/agent`) is used
by the engine; Pipkin shows status and offers a refresh. The installed engine runs on the system `node` and
uses `git` from `PATH`; `pipkin --diagnose` reports Node and the engine.

## Isolation

Tools run with the engine's permissions (as in Pi itself). Pipkin does not sandbox them and does not claim to.
