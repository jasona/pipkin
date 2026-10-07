# Native gate testing notes (owner-run)

**v1 qualification is active:** see [v1-release-gates.md](v1-release-gates.md) for the current identified build,
automated package evidence, owner-reported daily-use beta, and the release tracker. The beta below is no longer
“not started,” but individual manual checks still need recorded results; heavy use does not implicitly pass them.

These checks need a person at the machine and a real desktop session. They are **unverified** unless a result is recorded below. Launch with `scripts/try-m2.sh` (managed engine, scripted provider), or `cargo run -p pipkin-app --release -- --demo normal`.

| # | Gate | Steps | Pass when | Result |
|---|------|-------|-----------|--------|
| 1 | Minimize/restore | Open a conversation with text in the composer, minimize, restore. | Focus returns to the composer, draft intact, transcript scroll position unchanged, no blank frame. | unverified |
| 2 | Suspend/resume | Same setup with a run in progress (send a prompt first); suspend, resume. | App redraws; engine reconnects or shows "Not connected" with the saved copy; no duplicate send; draft intact. | unverified |
| 3 | Cold boot | Reboot, launch Pipkin. | Reopens the last conversation from the cache; time to usable window noted. | unverified |
| 4 | Screen reader (Orca) | `orca` running; focus the composer (Ctrl+L). Type text, move the caret with arrows and Ctrl+Left, select with Shift+Left. Then tab through navigation, transcript, tool blocks, run status. | Typed characters and caret moves are spoken; every control has a name and role; run status changes are announced. Known: on focus Orca reads the composer text, but typed characters and caret moves were **not** spoken in the last attempt (caret events arrive, no speech). | partial |
| 5 | IME | fcitx5 + an engine (pinyin worked once). Compose, pick a candidate, commit; also cancel with Esc. | Candidate window follows the caret; Enter during composition does not send; cancel leaves no stray text. Only pinyin commit has been observed. | partial |
| 6 | 150% scale | Set the display to 150% (Hyprland may snap to 1.6). Check light and dark themes, enlarged text, focus rings, a narrow window. | No clipped text or controls; focus ring visible; layout usable. One capture at 1.6 looked correct. | partial |
| 7 | Presentation latency | Needs a high-speed camera or compositor timing. | Not measurable from inside the app. | unverified |

## Install gates (need sudo, so owner-run)

| # | Gate | Steps | Pass when | Result |
|---|------|-------|-----------|--------|
| 8 | Clean install | `cd packaging && makepkg -d -f`, then `sudo pacman -U pipkin-*.pkg.tar.zst` on a machine or fresh user with no Pi checkout. | Pipkin appears in the launcher (Walker/rofi) with its icon; opening it shows the window; `pipkin --diagnose --probe` ends with no problems; a conversation with a real provider works. | unverified |
| 9 | Upgrade | Install again over it (bump `pkgrel`); open Pipkin with a saved draft first. | Draft and history are still there; `--diagnose` shows the new engine. | unverified |
| 10 | Rollback | `sudo pacman -U` the older package after the upgrade. | If the schema changed, Pipkin says the database is newer and stays untouched; restoring the backup works. | unverified |
| 11 | Long soak | `PIPKIN_SOAK_ROUNDS=20000` (see docs/packaging.md), and the real app open for a day. | Memory and open files stay flat. | partial (1500 prompts run) |

## Platform and release gates (M6)

| # | Gate | Steps | Pass when | Result |
|---|------|-------|-----------|--------|
| 12 | Drag and drop | Drag two files and a folder from a file manager onto the conversation. | The two files appear as attachment chips; a message says folders were skipped; the area tints while dragging. Needs a real drag, so it was only built, not tried. | unverified |
| 13 | Engine unavailable | Start with `--pi-repo /nonexistent`, then open a saved conversation. | A red strip says the engine is not available and to run `pipkin --diagnose`; sending is off. | unverified (built only) |
| 14 | X11 session | Log into an X11 session (or use Xwayland), run Pipkin; try the IME and Orca. | Renders, types, resizes; IME and screen reader work. Rendering was seen under Xwayland only. | partial |
| 15 | Other compositors | Run on GNOME and KDE Wayland: file picker, clipboard, decorations, IME. | Everything in the walkthrough works. | unverified |
| 16 | Generic installer on another distribution | `./install.sh` from the tarball on Fedora/Debian/Ubuntu using packaged Node (without system Node); upgrade, rollback, uninstall. | Same results as `scripts/test-install.sh`, and a launcher entry works. | unverified (checked on Arch in a scratch prefix) |
| 17 | Signed release | Sign with a real key, download elsewhere, run `verify-release.sh --require-signature`. | Verifies; a tampered file fails. | partial (throwaway key only) |
| 18 | Beta | See `beta.md` and `v1-release-gates.md`. | Targets met with recorded evidence. | owner reports heavy daily use; quantitative targets and other-user qualification remain open |

Record results here with the date, compositor, scale and input-method engine used.
