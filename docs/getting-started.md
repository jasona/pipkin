# Getting started with the qualification build

Pipkin is still an unsigned, unpublished **0.0.1 qualification build**, not an approved v1 release. The working
release target is x86_64 Arch/Omarchy with Hyprland/Wayland. Other desktops and pure X11 remain experimental.
See [release gates](v1-release-gates.md) for automated, observed, failed and unverified results.

## Recommended: one-command source setup

After the [native prerequisites](source-setup.md#1-install-native-build-prerequisites-once), run
`scripts/setup.sh --run` from this checkout. It obtains private Node/npm, fetches the pinned Pi source,
includes Pipkin's OAuth and default-enabled subagent patches, builds and offline-checks the paired app,
then launches with this checkout as the project. `scripts/run.sh` starts the last successful build;
`scripts/setup.sh --install` adds a per-user installation on Mac or Linux. No manual engine/server setup
or global Node installation is needed. See [source setup/self-development](source-setup.md).

## Advanced: build a pacman-managed Arch package

Before starting, ensure Git, Node.js 22.19+, npm and zstd are available; install the Arch build dependencies listed
in `packaging/PKGBUILD` through your normal package-management workflow. The preparation commands need these tools
before `makepkg -si` can install missing declared build dependencies. Do not run the build as root.

Use a fresh engine directory rather than changing your daily-use Pi checkout. These commands fetch public source
and npm dependencies, but do not make model requests or configure credentials:

```sh
git clone https://github.com/last-refuge/pipkin.git
cd pipkin
git clone --no-checkout https://github.com/jasona/pi.git ../pipkin-release-engine
git -C ../pipkin-release-engine checkout --detach "$(<packaging/pi-engine-revision)"
(cd ../pipkin-release-engine && npm ci)
scripts/stage-model-data.sh ../pipkin-release-engine
scripts/check-engine-source.sh ../pipkin-release-engine
(cd packaging && PI_CHECKOUT="$(realpath ../../pipkin-release-engine)" makepkg -si)
pipkin --version
pipkin --diagnose --probe
```

Package build dependencies are in `packaging/PKGBUILD`; the pinned Rust toolchain is in
`rust-toolchain.toml`. The installed package includes a private, pinned Node 22 runtime; no system Node is needed to *run* it. A functioning Vulkan-capable driver, Git and the desktop libraries declared in the PKGBUILD are still needed. A successful build is not clean-machine desktop qualification.

When a signed release is available, use its download/verification instructions instead of building mutable source.
Do not treat the current unsigned tarball as authenticated merely because its checksum matches.

## First run and provider sign-in

From a newly installed package, open Pipkin from its desktop launcher or run `pipkin`. The shared first-run GUI
welcomes you, offers **Claude Pro/Max** and **ChatGPT Plus/Pro** OAuth sign-in, then asks for a project folder and
an available Pi model. The app stores no sign-in answers; Pi stores credentials in its agent profile (normally
`~/.pi/agent`, or `--pi-agent-dir DIR`). Sign-in, project and model selection are not commands to paste into the
chat composer. A returning profile with saved work or Pi credentials opens its existing workspace rather than
forcing a new login. Keep Pi's `auth.json` private; never include it in a report.

The wizard does **not** set up API keys or other providers. Advanced users may configure Pi directly and keep their
existing credentials; an API-key environment variable set in a terminal is not necessarily inherited by a desktop
launcher. If a separate/external engine lacks Pipkin's tested OAuth service, the wizard does not offer usable-looking
login buttons. The installer cannot authenticate your provider or prove that a paid model request succeeds.

After selecting a model, send a small prompt. **This makes a real provider request and may be billed.** Demo mode
is simulated; the scripted-provider test is offline. Neither establishes that your paid provider works.

### Missing or expired authentication

- No usable models: open **Ctrl K → Refresh models** (or the empty-state Refresh models button). Refresh is an
  engine catalog operation, not provider login or a guarantee of network access.
- Missing key/unauthorized/expired token: read the visible failure; check the selected provider and the agent
  directory used by the engine. Reauthenticate with Pi or renew the provider key, then reconnect/restart as needed.
- Models still missing: check provider configuration and network/proxy access. Do not delete saved sessions or the
  desktop database to fix credentials. Do not repeatedly resend a file-changing prompt as an authentication test.
- `pipkin --diagnose --probe` proves an offline handshake, **not** provider authentication or account balance.

The setup workflow is documented from the implementation and Pi provider contract. A new-user installation to
real first reply on a clean supported desktop remains an open qualification gate; no participant results are claimed.

## Know what the controls mean

- **Stop** retires stopped input from future model context in the pinned engine; history and completed tool effects
  remain. It is not undo. A raced successful completion may remain completed.
- **Steer/queue** are engine-authoritative. If settlement is unknown, wait for reconciliation rather than copying
  and resending the prompt. Timeout does not prove a tool stopped.
- **Changes** shows the current uncommitted Git diff for files associated with the selected session's successful
  `edit`/`write` calls. It includes staged, unstaged, and untracked changes—not a history of tool operations.
  Committed or reverted changes disappear, and subsequent edits compare against the latest HEAD. A lightweight
  background scan detects external commits too. Session file scope excludes unrelated project files; if multiple
  tools edit the same session-owned file, its diff reflects that file's current combined uncommitted state.
  Refresh does not send agent input. Earlier session file paths are recovered from bounded engine history.
- **Prompt history** is separate for each session. In the conversation input, Up recalls older prompts when the
  caret is on the first visual line; Down moves toward newer prompts on the last visual line. Moving past the newest
  prompt restores your unsent draft. Editing a recalled prompt starts a new draft rather than changing history.
  The latest 200 journaled prompts survive app restarts; loaded conversation messages also populate recall history.
  Recall changes text only: it never sends a prompt or reattaches old files. Selection, IME composition, and `@` path
  completion retain their normal arrow-key behavior.
- **Search** covers locally cached history, not all engine history. Older material may not yet be cached.
- **`@` paths** are project-scoped references inserted into the text. Selecting one does **not** attach file contents.
  Use the attachment picker/drag-and-drop when you want an attachment; missing/deleted paths cannot become files by
  being mentioned. Tab/Enter or a popup click inserts the selected reference; Ctrl-click opens a linked path.
  Each newly opened mention query refreshes a bounded, read-only path snapshot off the UI thread. It is not a
  live filesystem watch or an exhaustive index: dependency/VCS folders, symlinks and unreadable descendants are
  skipped. Newly created/deleted files appear on the next query refresh. A failed root scan is shown as unavailable,
  not an empty success; repair access or select the correct project, then reopen the query to retry.
  Native picker/drop/IME qualification remains separately tracked.
- **Extensions**: only the experimental remote question contract is supported. Stable Pi `ctx.ui.*` and terminal
  widgets are not bridged; see [the compatibility matrix](extensions.md).

See [support and recovery](support.md) for data ownership, quitting and diagnostic privacy.
