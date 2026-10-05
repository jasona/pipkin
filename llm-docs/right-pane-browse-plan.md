# Right-pane Browse tab — implementation plan

Status: proposed; planning only. This is not an implementation or a claim that GPUI can host a webview on the supported desktop. The target release track is Arch/Omarchy on Wayland; X11 and other desktops need separate qualification.

## Outcome and scope

A link in a conversation should open a readable document beside that conversation, without losing the transcript or the Changes inspector. The right pane gains two top-level tabs, **Changes** and **Browse**. Browse holds a small, bounded set of document tabs. This is a document preview first; calling it an in-app *browser* and routing arbitrary websites into it depends on the embedded-web-engine gate below.

The initial supported set is project-local Markdown (`.md`, `.markdown`), plain text/source files (validated UTF-8, with line/size bounds), and common images (PNG, JPEG, GIF, WebP). PDFs, HTML, unknown binaries, mail links, and `http(s)` open externally until their own preview engines meet the gates. A preview must never silently substitute a different file for a missing one. Provide **Open externally** in the preview header.

### Current implementation to retain

- `crates/pipkin-ui/src/shell/inspector.rs` renders the Changes inspector, including the virtualized diff and changed-file list. Keep its selection and scroll state while Browse is active; do not rebuild it by treating a preview as a file change.
- `crates/pipkin-ui/src/shell/workspace.rs` owns docked/temporary right-pane behavior and persisted width. Browse uses that same pane and the existing temporary drawer at narrow widths. Do not add a second sidebar or impose a new fixed maximum width.
- `crates/pipkin-ui/src/transcript/view.rs` currently handles Markdown-link clicks and inline-code references separately. `crates/pipkin-ui/src/transcript/links.rs` validates project-local links, canonicalizes paths, checks symlink escapes, and produces external `file://` URIs. Evolve that resolver into a destination classifier rather than duplicating its validation in the view.
- `pipkin-core` has `SelectChange` and `selected_change` for the Changes inspector. Browse navigation is view state, not a backend operation. Do not mutate the selected change to show a preview. Preserve the `Demo · simulated agent` label.

## Interaction contract

1. Clicking a supported local link in the transcript selects or creates its document tab and switches the right pane to Browse. If the pane is hidden, reveal it; if it cannot dock, open the existing right drawer. Keep the transcript scroll, selection, and composer draft unchanged.
2. Clicking another supported local link selects its existing tab by canonical project path or opens a new one. A link clicked *within* a preview resolves relative to that document's parent directory, not to the project root. `#fragment` should scroll to a heading in Markdown when feasible; until implemented, do not claim fragment navigation works.
3. The top-level tab bar is **Changes | Browse**, with readable selected/focus states and accessible names. Browse's document tab strip has a filename, close action, overflow handling, and a visible active tab. Do not let many tabs reduce the content area to zero; cap the set (suggested: eight) and use a predictable eviction/confirmation policy that never closes a tab holding user edits (previews are read-only).
4. Returning to Changes restores its selected file and scroll; returning to Browse restores the active document and its scroll. Switches between conversations in the same project may retain document tabs during this window's lifetime; switching projects must not present stale files under the new project's identity. No persistent tab/content storage in the first release.
5. Close a document tab without closing Changes; when the last Browse tab closes, switch to Changes. Escape closes the temporary drawer according to existing behavior. Keyboard focus returns to the originating link when possible; tab activation and closing have shortcuts and visible focus. A link that cannot be opened presents a specific, actionable error rather than silently doing nothing.
6. A local link in a saved/offline conversation uses that conversation's project identity, not whichever project is currently selected. If the project folder is unavailable, show that state and offer Open externally only where safe. An external web link still opens the system browser in phase one.

## Architecture and data flow

- Define a typed navigation request/result, for example `OpenLink { conversation, source_document?, raw_target }` → `LocalPreview { project, canonical_path, kind, fragment? } | ExternalUrl | Unsupported(reason)`. The transcript emits the request to the workspace (or a dedicated navigation coordinator); it does not call `cx.open_url` for destinations that belong in Browse. Reuse the existing URL/path decoding and safety checks. Do not infer project identity from a later selection after an asynchronous read.
- Store pane mode and document tabs in a GPUI UI entity. Keep at most a small number of previews and bound memory. `pipkin-core` remains GPUI-free and authoritative for backend state; add a core command only if later persistence or cross-window semantics genuinely require it.
- Perform file reads, MIME/signature checks, decoding, Markdown parsing, and image preparation off render. Each load carries project, canonical path, tab id and revision/generation. Apply a result only if that tab still exists and the request is current; closing a tab or switching projects must not show a late file in the wrong tab. No render-time I/O or parsing.
- Reuse the transcript's Markdown building blocks only where their selection, wrapping and link semantics suit a document view. Avoid putting an entire document into one transcript item or repeatedly reparsing it on every frame. Use a virtualized/scrollable document view for long files and retain per-tab scroll and selection. Provide copy and keyboard navigation; keep preview read-only.
- Set explicit caps before implementation: max file bytes and decoded pixels, text line length/count, Markdown blocks, open tabs and cached previews. Above a limit, say why and offer Open externally. Surface missing, permission-denied, invalid encoding, changed-on-disk, unsupported format, and load-cancelled states. Consider refresh-on-demand for a file changed after opening; never silently replace selected text mid-read.

## Security boundary

Agent-authored Markdown and project files are untrusted. Resolve and canonicalize local paths before reading; enforce containment in the conversation's project, including symlinks and `..`, and recheck at read/open time to avoid stale assumptions. The current resolver's external `file://` generation is not itself a permission boundary. Reject `javascript:`, `data:`, remote file authorities and unapproved custom schemes. Rendering Markdown must not execute HTML/scripts or automatically fetch remote images. Never expose credentials or arbitrary files to a webpage because it was opened from a conversation. File navigation requires an explicit click, not automatic loading of every link in a message.

For any eventual webview: use separate unprivileged web content, a restrictive navigation policy, no access to the project's files or app APIs, and a partitioned/ephemeral profile unless the owner approves persistent web state. Define download, popup, authentication, permission, clipboard and external-protocol behavior before enabling remote sites. A website must not gain `file://` access through the preview's local-document origin.

## Milestones and gates

### 0. Interaction and feasibility prototype

Sketch the tab bar at docked width and narrow-drawer width. Test navigation, selection, focus restoration and two switched documents using temporary state. Confirm the current GPUI version's facilities for image display and document scrolling. Identify any new runtime/package dependencies before choosing a web engine.

### 1. Project-local preview

Implement classifier, request routing, right-pane tabs, read-only Markdown/text/source and image previews, limits, errors and Open externally. Test relative links from transcript and from a document, UTF-8 filenames/spaces, fragments, missing paths and symlink escapes. Keep unsupported types and all remote URLs external. This milestone is useful without an embedded browser.

**Exit:** Clicking `docs/native-gate-testing.md` in a conversation opens the correct local document in Browse without a URI error; a second document creates/selects a tab; Changes restores its diff and file-list scroll; narrow windows use the existing drawer. No render-time disk work.

### 2. Embedded-browser go/no-go spike

In an isolated branch, evaluate a maintained WebKit-based integration (or another viable engine) *inside this GPUI Wayland/X11 window*. Prove it embeds rather than launching a separate window; follows right-pane resize and dock/drawer transitions; handles keyboard focus, IME, clipboard, accessibility and high-DPI; tears down cleanly; and packages/launches from a clean install. Measure memory/process cost and audit navigation, file access, network, downloads and permissions. Do not claim GPUI has built-in webview support or mark the spike complete because a separate GTK window can render a page.

**Go:** all of the above work with an acceptable dependency/security footprint on the claimed platform. **No-go:** ship milestone 1 as **Preview**, keep `http(s)`, local HTML and PDF external, and document the limitation. A working screenshot alone is not a go decision.

### 3. Web navigation (only after a go)

Route opted-in `http(s)` to a web tab with loading/error states, back/forward/reload, address/origin display, Open externally, explicit policy for popups/downloads and blocked schemes. Decide whether local HTML can be shown without granting it project access. PDF preview is a separate decision, not an automatic consequence of web support. Respect the existing app's accessibility, IME and packaging gates before calling this a supported in-app browser.

## Validation matrix

- **Functional:** relative and absolute local links, URL-encoded spaces/Unicode, nested Markdown links, duplicate links, anchors, external `http(s)`/`mailto:`, unsupported types, deleted/renamed files, changed-on-disk files, cross-conversation/project switches, late async loads, tab cap and close behavior.
- **Security:** `../` and symlink escape, remote `file://` authority, encoded traversal, unsupported schemes, Markdown HTML/remote assets, oversized/decompression-bomb images, webview origin isolation (if enabled).
- **Native UI:** light/dark, text scale, docked and drawer widths, dragged divider, keyboard-only operation, focus return, copy/selection, AT-SPI/Orca and IME, repeated open/close memory and file descriptor behavior. Screen-reader and IME checks need real native testing; source-level support alone is not a pass.
- **Release:** `cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --all --check`; a clean packaged launch with any new runtime libraries; record measured, observed and unverified results separately in the native gate notes. Synthetic input, if used for native verification, must go only through `scripts/guard.sh` under its safety rules.

## Decisions to confirm before implementation

- Are remote `http(s)` sites essential for the first release, or is local-document preview plus external web links an acceptable first slice? This plan recommends the latter pending the webview spike.
- Should document tabs be scoped per project (recommended), per conversation, or globally in a window?
- Which source formats beyond Markdown/text/images should receive first-class read-only preview? PDF and local HTML carry separate rendering/security costs.
