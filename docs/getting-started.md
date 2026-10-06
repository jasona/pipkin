# Getting started with the qualification build

Pipkin is still an unsigned, unpublished **0.0.1 qualification build**, not an approved v1 release. The working
release target is x86_64 Arch/Omarchy with Hyprland/Wayland. Other desktops and pure X11 remain experimental.
See [release gates](v1-release-gates.md) for automated, observed, failed and unverified results.

## Build an identified package

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
`rust-toolchain.toml`. Runtime needs Node.js **22.19 or newer**, a functioning Vulkan-capable driver, and the desktop
libraries declared in the PKGBUILD. A successful build is not clean-machine desktop qualification.

When a signed release is available, use its download/verification instructions instead of building mutable source.
Do not treat the current unsigned tarball as authenticated merely because its checksum matches.

## Configure a provider

Pipkin does not currently provide a provider login dialog. It uses the Pi agent configuration (normally
`~/.pi/agent`, or `--pi-agent-dir DIR`) and credentials/environment of the process that starts its owned engine.
Use existing Pi credentials, or Pi's `/login` in its terminal client with the **same agent directory**. `/login` is
not a Pipkin composer command. Keep Pi's `auth.json` private; never include it in a report.

For an API-key provider, an environment variable is another option. For example, in Bash, this avoids putting the
literal key in shell history or echoing it to the terminal:

```sh
read -rsp 'Anthropic API key: ' ANTHROPIC_API_KEY; printf '\n'
export ANTHROPIC_API_KEY
pipkin --project /path/to/your/project
unset ANTHROPIC_API_KEY
```

OpenAI uses `OPENAI_API_KEY`; provider-specific setup and OAuth are described in the pinned engine's
`packages/coding-agent/docs/providers.md`. Do not commit keys or paste them into a conversation. Environment
credentials are visible to processes/tools that inherit them; only run trusted projects and extensions. A desktop
launcher does not necessarily inherit variables exported in a terminal: use stored Pi credentials or launch from
that terminal. Changing the terminal environment does not update an already running external server.

Select a model with **Ctrl M**, then send a small prompt. **This makes a real provider request and may be billed.**
Demo mode is simulated; the scripted-provider test is offline. Neither establishes that your paid provider works.

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
- **Changes** describes the project's current Git working tree, not only edits attributed to the selected chat.
  Other chats, your editor, and external commands in the same project can affect it. Refresh failures preserve a
  stale previous result with a reason; a stale result is not a new successful scan.
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
