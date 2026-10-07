# macOS application menu and lifecycle

Pipkin uses the native macOS menu bar, not an in-window imitation. The Mac-only integration in
`crates/pipkin-app/src/native_menu.rs` provides:

- **Pipkin:** About Pipkin, Settings… (⌘,), system Services, Hide Pipkin (⌘H), Hide Others (⌥⌘H),
  Show All, and **Quit Pipkin (⌘Q)**.
- **File:** New Conversation (⌘N), Open Project… (⇧⌘O), Close Window (⌘W).
- **Edit:** Undo/Redo, Cut/Copy/Paste and Select All, using the existing editor/selection actions.
  Mac Command-key editing shortcuts work in the composer and Command-C/A work in the transcript.
- **Window:** Minimize (⌘M), Zoom, Show Pipkin.
- **Help:** Pipkin Help opens the getting-started guide.

## Close is not Quit on a Mac

Closing the red window button or choosing Close Window closes the window, **not the application**.
This is normal macOS behavior. The live model is retained so drafts, history and running work are not
lost when the window closes. Clicking the Dock icon or choosing Window → Show Pipkin reopens the same
workspace without starting another engine or reinitializing the application. Settings and About can
also restore a closed workspace. Window/edit commands are disabled when there is no window.

**Pipkin → Quit Pipkin** or **⌘Q** exits the application, including with no window open. Both use GPUI's
normal `quit()` path and the controller's existing orderly shutdown hook: flush dirty drafts/cache,
stop the backend/only an engine owned by this app, then drain/join storage. No force-exit or process-name
kill is added. macOS quit mode remains GPUI's existing explicit-quit default.

## Cross-platform scope

Menu installation, Dock reopening and Command-key bindings are macOS-only. Linux retains its existing
Control-key bindings, in-app controls and quit-on-last-window behavior. No native Apple dependency is
introduced into the portable state core. The shared window helper only separates one-time setup from
recreating a window; normal initial startup is unchanged.

## Evidence and owner check

Automated tests check menu ordering, Services, the real Quit action, Command-Q mapping and closed-window
menu availability. Workspace tests, Clippy and formatting pass on Linux; Mac CI runs the menu tests and
compiles/packages the actual native app. These are not an observed native menu click or Dock interaction.

Native Apple Silicon build [37686573006](https://github.com/last-refuge/pipkin/actions/runs/37686573006)
passed on clean commit `5310b0a962b54ca38a419b7621b8c3edb0f59537`: all three menu contract tests,
actual native compilation, bundled engine lifecycle/edit/history smoke checks, ZIP extraction and DMG
copy-install signature/runtime probes. Both extracted and DMG-installed CLI probes reported engine ready
in **1.9s**. No native menu click, keyboard input or Dock interaction was automated or observed.

Download the [ZIP/DMG artifact](https://github.com/last-refuge/pipkin/actions/runs/37686573006/artifacts/11512137232)
(`pipkin-macos-ARM64-5310b0a962b54ca38a419b7621b8c3edb0f59537`). Replace the old app rather than merging
bundles. Signing remains ad-hoc, not Developer ID/notarized. Local workspace/Clippy logs are
`/tmp/pipkin-mac-menu-{workspace,clippy}.log`; native evidence is `/tmp/pipkin-mac-menu-ci-full.log`.
Concurrent onboarding/subagent changes were preserved and are not attributed to this menu milestone.

On the new Mac build, please verify:

1. Pipkin's menu opens, Settings/About work, and Hide/Hide Others/Show All behave normally.
2. Type a disposable draft. Close the window; use the Dock icon or Show Pipkin to restore it. The draft
   should remain, with no second workspace/engine created.
3. Quit using the menu with a window open; relaunch and check draft recovery.
4. Close the window and then use Pipkin → Quit Pipkin. The application should exit rather than remain
   in the Dock as running. Also verify ⌘Q and ⌘W do their distinct jobs.

Finder/menu interaction and on-device quit/reopen are owner-observed acceptance gates, not marked
passed merely because a build or CLI engine probe is green.
