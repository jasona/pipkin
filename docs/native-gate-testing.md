# Native gate testing notes (owner-run)

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

Record results here with the date, compositor, scale and input-method engine used.
