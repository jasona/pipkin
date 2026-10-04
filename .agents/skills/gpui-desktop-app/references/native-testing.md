# Native testing on Wayland / Hyprland (Omarchy) — and how not to hurt the user's desktop

## What worked (scripts in `scripts/`)
- Launch the release binary with a throwaway `--data-dir`; find it with `hyprctl clients -j` (class = your `app_id`).
- **Screenshots:** `grim -g "X,Y WxH" out.png` using the window's `at`/`size` from `hyprctl clients -j` (`scripts/shot.sh`). Read the PNG to inspect.
- **Keyboard:** `wtype "text"`, `wtype -k Return`, `wtype -M ctrl -k Return -m ctrl`, `wtype -s 30 "…"` for a 30 ms cadence (latency tests). Always through `scripts/guard.sh`, which refuses to type unless the active window class is yours.
- **Mouse:** `/dev/uinput` was writable. `scripts/mouse.py` builds a virtual **absolute** pointer (like a VM tablet: ABS_X/ABS_Y 0..32767, BTN_LEFT, REL_WHEEL) with raw ioctls, no `evdev` module needed. Commands: `move|click|dblclick|tripleclick|drag|wheel`. Coordinates are global layout pixels (edit `W,H` for your output). It enabled real drag-selection across rows, wheel scrolling mid-stream, and button clicks.
- **Resize:** Hyprland 0.56 uses Lua dispatch: `hyprctl dispatch "hl.dsp.window.resize({ x = DX, y = DY, relative = true })"` acts on the **active** window and is relative (`scripts/size.sh` computes deltas). Old `resizewindowpixel` syntax errors.
- **Accessibility:** `python3` with `gi.repository.Atspi`; find the app by name under `Atspi.get_desktop(0)` (our binary showed as `desktop-app`), walk children, print role/name.
- **Perf probes:** a palette command that logs frame p50/p95, input-to-frame latency and `VmRSS`; launch timing by polling `hyprctl clients` for the class (warm 175–218 ms; page-cache-evicted binary via `posix_fadvise(DONTNEED)` 197–258 ms; idle RSS 140 MB).
- **Persistence:** type, wait for "Draft saved", `kill -9`, relaunch with the same `--data-dir`: draft restored.

## Safety rules (learned the hard way)
1. **Another app stole focus mid-run** (a game window activated itself) and some keystrokes may have gone into it. Never batch input without a guard; check the active window before *each* send. Pointer events go to whatever window is under the pointer: keep coordinates in a region your window owns, and don't assume it's on top.
2. Do **not** move, resize, focus or workspace-switch windows you did not launch, even to "fix" overlap. A relative resize/move dispatch hits the *active* window, which may not be yours. If a harness denies a workspace switch, do not retry another way; ask. The user said: use the current workspace.
3. Don't edit the user's Hyprland/Omarchy config (opacity rules etc.) to improve screenshots. Omarchy's default window-opacity rule makes your window ghost other windows in captures; note it as a caveat instead.
4. Don't install packages (IME, Orca) or change display scale/suspend the machine without explicit approval; record the gate as unverified. We asked via a question and the user chose "accept as unverified".
5. `focuswindow`/`hl.dsp.focus` dispatch by address did **not** reliably focus our window; a newly launched window gets focus by default. Verify with `hyprctl activewindow -j` instead of assuming.

## Captures worth taking (a matrix)
Dark streaming + diff; light + diff; 1024 and 720 px (and dark 720 with nav drawer); palette; selection + paste; steer/queue/running → stopping → stopped with queue kept; scrolled-away with "Jump to latest"; empty state; 10,000-message history; persistence recovery. Not captured here: failure/recovery scenario, expanded-tool close-up, screen recording.
