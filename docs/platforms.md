# Platforms: what is supported, what was checked, what blocks the rest

**Working v1 qualification scope: x86_64 Arch/Omarchy, Hyprland/Wayland.** Final support/acceptance approval and
installed native gates remain open. See the current [release-gate record](v1-release-gates.md); successful headless
Ubuntu CI does not qualify an Ubuntu desktop or an additional supported distribution.

Native observations below include historical author-machine checks (Arch/Omarchy, Hyprland 0.56, Intel + NVIDIA,
125% scale). They are not a complete final-candidate matrix. Later scale and accessibility evidence is separately
recorded in [native gates](native-gate-testing.md); do not read that historical setup as the current display setting.

| Target | Status | Evidence |
| --- | --- | --- |
| x86_64 Omarchy/Arch, Wayland (Hyprland) | **Qualification build; final approval open** | Owner-reported daily use; current locked development/packaged engine checks and scratch installer checks passed. Historical 1500-prompt measurement is not final-candidate soak. Clean installed desktop/native gates remain open |
| Arch/other Linux, X11 (also Xwayland) | **Builds and starts; checked under Xwayland only** | The `x11` GPUI feature is enabled alongside `wayland` (one binary; the display server picks at run time). Launched with `WAYLAND_DISPLAY` unset on Xwayland: it renders correctly. A pure X11 session, other window managers, and IME/screen reader on X11 were not tried |
| Other Wayland compositors (GNOME, KDE) | **Unverified** | Same Wayland code path as Hyprland, but portals (file picker), decorations, clipboard and IME were not tried on them |
| Other Linux distributions | **Experimental; native install/runtime unqualified** | Generic installer mechanics passed in scratch prefixes, not a distribution desktop matrix. Native ABI and external libraries vary by artifact; the local app references GLIBC_2.44. Ubuntu 24.04 headless CI builds its own artifact and is not proof that the local binary runs there |
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

## Engine compatibility and runtime requirements

Packages pair the app with the exact Pi revision in `packaging/pi-engine-revision`; current protocol is **8** and
app database schema is **7**. An external server must satisfy the app's compatibility checks; arbitrary/latest
Pi revisions are not promised compatible. Requalify any changed pair, and do not bypass a protocol/schema refusal.

Runtime requirements include **Node >=22.19**, Git, a working Vulkan-capable graphics setup and desktop libraries
listed in [PKGBUILD](../packaging/PKGBUILD). The bundled engine is not a bundled system Node or GPU driver.
Inspect the artifact's [native/runtime inventory](bundled-licenses.md#native-runtime-inspection) for direct ELF
library/ABI references. It is not a complete static/dlopen dependency audit or portability guarantee. Optional
native VM/sandbox components and foreign source-tree artifacts remain provenance/runtime review findings; their
presence does not extend platform support or promise a sandbox.

## Credentials and tool discovery

Pipkin does not provide its own credential store. Its logs/caches can still contain sensitive provider/tool/error
text and must not be assumed credential-free. Pi's own mechanism (login, API keys, `models.json` under `~/.pi/agent`) is used
by the engine; Pipkin shows status and offers a refresh. The installed engine runs on the system `node` and
uses `git` from `PATH`; `pipkin --diagnose` reports Node and the engine.

## Isolation

Tools run with the engine's permissions (as in Pi itself). Pipkin does not sandbox them and does not claim to.
