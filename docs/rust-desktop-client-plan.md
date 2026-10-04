# Rust Desktop Client for Pi: prototype to working product

Updated: 2026-10-04. Status: implementation roadmap. M0 is done and M1 is met against a real Pi server, with caveats; see "Implementation status" near the end of section 10 for exactly where to resume.

## 1. Continue the application that exists

Build the real Pi client inside the existing **Pipkin Rust/GPUI application**. Retain its composer, virtualized transcript, selection model, themes, workspace, command registry, pure state core, draft writer, and deterministic demo scenarios. The next work is engine integration and production correctness, followed by product completion and platform qualification.

The [prototype plan](gpui-prototype-plan.md) records the original build scope. Its opening “application not yet built” status is historical: the code, [architecture](architecture.md), [continuation map](continuation-map.md), and [scorecard](scorecard.md) establish the implemented baseline. This document supersedes the original product plan's framework competition, three visual studies, and disposable RPC prototype milestones. Continue the direction in [DESIGN.md](DESIGN.md).

GPUI has a **conditional go on the tested Omarchy/Hyprland/Wayland machine**, not general platform approval. Preserve the exact GPUI revision in [Cargo.toml](../Cargo.toml), `a84689073d296dfd39987bc7dd478e43ef76d83a`, and the recorded Rust toolchain until a deliberate dependency change is justified. Close the outstanding text, accessibility, scaling, and lifecycle checks during integration. Reopen the framework decision only if an essential requirement cannot be met at a bounded maintenance cost.

The product remains a local developer workspace:

**Open a real project → create or resume a conversation → send a prompt → observe actual model/tool work → inspect workspace changes → steer, queue, or stop → reopen without losing context or duplicating work.**

First ship an Omarchy/Wayland daily-use alpha. Broader Linux, macOS, and Windows support remain explicit later qualification tracks. Defer cloud/team features, a marketplace, a full editor, an embedded terminal, and automatic patch reversion. Provide external editor and terminal launching in the local workflow.

## 2. Source-grounded starting point

This revision reviewed Pipkin at `807391929ba7a409b1406d60da60ec64a4065103` and the adjacent Pi checkout at `200387122ca450d6387f033949423114a270b96c`. These identify inspected baselines, not a shipped compatibility promise. Record the tested engine revision and service-contract version in a build manifest before integration.

Pi is a separate repository, currently available at `../../pi` relative to this document. The links below point into that checkout; they are not Pipkin package directories or runtime dependencies. A clean build must obtain a pinned engine artifact/source independently of that directory layout. The continuation map's statement that engine sources are absent from this workspace is still true for Pipkin itself; adjacent sources were available for this review.

### What the prototype actually implements

| Existing code | Preserve | Production gap |
| --- | --- | --- |
| [Core state](../crates/pipkin-core/src/state.rs), [commands/events](../crates/pipkin-core/src/protocol.rs) | Commands → effects, shared availability, generation guards, stopping/unknown/save states | No connection lifecycle or capability state; operations and items use process-local counters; create/rename and queue management are local |
| [Backend port](../crates/pipkin-core/src/backend.rs) | Application-level boundary independent of GPUI and transport | Synchronous `bootstrap()` and fire-and-forget `request()` need asynchronous initialization, errors, shutdown, and richer snapshots |
| [Controller](../crates/pipkin-app/src/controller.rs) | Bounded event channel and token coalescing | Always constructs `DemoBackend`, restores demo conversations, installs demo globals; no engine host or real startup path |
| [Storage](../crates/pipkin-app/src/storage.rs) | Versioned SQLite, single bounded writer, commit acknowledgments | Drafts persist text only, keyed by numeric conversation ID; no durable request journal, attachments, or real-session namespace; fallback/load failures are logged rather than fully represented in UI |
| [Composer](../crates/pipkin-ui/src/text/), [transcript](../crates/pipkin-ui/src/transcript/) | Editing, Markdown/code, document-level selection, virtualization, anchoring | Real IME and screen-reader gaps; real backend content, bounded caches, and large-output retrieval still need integration |
| [Workspace](../crates/pipkin-ui/src/shell/) | Navigation, overlays, themes, model menu, tool and diff views | Models/projects/diffs are fixtures; search matches loaded conversation titles; attachments are file references, not delivered model content |
| [Demo adapter](../crates/pipkin-app/src/adapters/demo.rs), [scenarios](../fixtures/scenarios/) | Deterministic regression backend and fault scenarios | Demonstrates client behavior only; cannot establish real deduplication, tool execution, or reconnect correctness |

The scorecard reports 135 passing prototype tests and partial native verification. It records roughly 140 MB idle RSS, 175–218 ms warm window mapping, and p95 input-handler-to-paint of 4.8 ms. These are historical prototype measurements: window mapping is not usable startup, paint submission is not display presentation, and engine memory is not included. Re-measure the integrated application.

### What Pi already supplies, and what it does not

Inspect these contracts before changing either repository:

| Pi source | Available baseline | Required continuation |
| --- | --- | --- |
| [Protocol](../../pi/packages/protocol/README.md), [client](../../pi/packages/client/README.md) | Protocol v8 framed CBOR, server/session routes, attachment fencing; Chord service snapshots and updates | Experimental, no compatibility guarantee; peer authentication is not implemented; Rust must implement the required Chord semantics as well as envelopes |
| [Session contracts](../../pi/packages/coding-agent/src/experimental/services/sessions.ts) | List, create, remove, attach, detach | Summaries expose server/session IDs and creation time, not title/cwd/activity; creation options lack project cwd; add project metadata and rename |
| [Agent controller](../../pi/packages/coding-agent/src/experimental/services/agent-controller.ts) and [provider](../../pi/packages/coding-agent/src/experimental/services/agent-controller-provider.ts) | Prompt, steer, follow-up, cancel queued entry, abort, compact, wait for known prompt ID | No client idempotency key or lookup by client request ID; distinguish operation acceptance from settlement; define stop/queue semantics |
| [Models](../../pi/packages/coding-agent/src/experimental/services/models.ts) | Provider/model identity, catalog, selection, thinking levels, refresh | Native credential onboarding and auth status contract; model selection is session service state, not a field on `prompt()` |
| [Transcript](../../pi/packages/coding-agent/src/experimental/services/transcript.ts), [service overview](../../pi/packages/coding-agent/src/experimental/services/README.md) | Durable conversation view, including active entries and live/inbox/agent/usage documents | Complete history before reset/compaction needs paging over durable entries; root conversation only, no tree/subagent navigation contract |
| [Durable runtime](../../pi/packages/durable/README.md) | Worker-owned execution and persistence | Verify process/power-loss guarantees; package experimental server/worker paths, which are currently excluded from published packages and standalone binaries |

Do not assume the demo's `Token`, `ToolStarted`, or `StatusResolved` variants exist on the wire. Pi publishes replicated service state. The adapter must translate that state and its revisions into stable presentation updates.

## 3. Architecture and ownership

Keep the three existing crates. Add transport and host modules under `pipkin-app` first; extract a reusable `pi-client` crate once the conformance harness establishes a useful boundary. Do not split storage, platform, and testing into new crates merely to match the old suggested layout.

```mermaid
flowchart LR
    UI["Existing GPUI workspace"] --> Core["pipkin-core: commands and state"]
    Core --> Controller["pipkin-app: effects and lifecycle"]
    Controller --> Store["Desktop SQLite: drafts, request journal, preferences, cache"]
    Controller --> Demo["DemoBackend: explicit demo mode"]
    Controller --> Adapter["PiBackend: service mapping and reconciliation"]
    Adapter --> Client["Rust protocol and Chord subset"]
    Client <-->|"Authenticated local connection"| Engine["Managed Pi server and session workers"]
    Engine --> Durable["Authoritative history, queues, models, tools, extensions"]
```

Proposed modules, not existing implementations:

- `adapters/pi/`: map Pi contracts into core snapshots/events and commands.
- `pi_client/`: framing, handshake, routing, service calls, subscriptions, replica decoding.
- `engine_host/`: resolve/launch the pinned engine, readiness, discovery, shutdown, bounded recovery.
- `workspace_changes/`: read-only project Git status/diff service unless Pi gains that capability.
- Integration fixtures/harness: real engine with fake providers, transport faults, process termination.

Ownership rules:

1. Pi owns session identity, authoritative history, model configuration, submitted input, queued input, and execution state. The desktop never writes worker-owned databases.
2. Pipkin owns project bookmarks, drafts, unsent attachment metadata, preferences, request intents, UI selection/scroll state, and rebuildable offline/search caches. A cached transcript is explicitly stale when disconnected.
3. The core remains free of GPUI and wire types. Views dispatch commands; controller tasks execute effects. Rendering does no process, storage, transport, or parsing work.
4. Local IDs may remain internal handles, but must map to stable, namespaced engine identities. Preserve `(conversation, generation, op)` guards and add server/session/attachment routing and replica revision checks.
5. Background queues, frame sizes, pending requests, output previews, parsing work, and caches have explicit bounds. Overflow must cause backpressure or a visible resync/error; never silently lose authoritative state updates.

Use one production backend: the durable Pi service architecture. Keep the demo adapter as a test/development backend. Do not add a second production implementation based on legacy JSONL RPC to bypass missing service work.

## 4. Make the backend port suitable for real services

Evolve the application contract before connecting live tools. Keep demo behavior working through the same port.

| Application capability | Required change |
| --- | --- |
| Startup | Replace blocking complete bootstrap with asynchronous connection/catalog/session initialization. Show connecting, ready, reconnecting, offline, incompatible, and failed states independently of run/save states |
| Projects and conversations | Add open/register project and backend create/rename operations with pending/success/failure results. Map one desktop conversation to one Pi Session/root conversation initially; defer branching/subagent navigation |
| Opening and switching | Hydrate history **and** run/queue/model state. Detach or retire subscriptions deliberately. Pi currently has one selected Session attachment per client connection: start with one active attachment and rehydrate on switch; do not imply continuous live updates for detached sessions |
| History | Use opaque backend cursors and stable item IDs, not `before: Option<ItemId>` arithmetic. Add page request identity, loading/error/retry, deduplication, and cancellation on switch |
| Submit and steer | Persist request identity and immutable payload before transport. Correlate acknowledgments to the exact submission/steering item, not the latest user row |
| Follow-ups | Add backend enqueue/remove requests and authoritative queue snapshots. Preserve text, attachments, and agreed model semantics; handle `already_consumed` races |
| Status | Replace `StatusResolved { accepted: bool }` with pending/running/completed/failed/cancelled/definitely-not-accepted/unknown outcomes and durable operation identity |
| Replicated output | Upsert by stable message/tool IDs and revisions. Reconcile optimistic rows with authoritative entries; snapshot replacement and replay must not append duplicate tokens or tools |
| Models/capabilities | Derive action availability from engine capabilities, connection state, model/auth readiness, and core run state. A menu change updates `Models.select()` and waits for authoritative state |
| Changes and large results | Real asynchronous diff refresh and bounded full-output retrieval/export; loading, stale, unavailable, and error states |

Expand the core for unknown content kinds, thinking/usage/error information, compaction boundaries, and pending input as supported by the inspected `ConversationView`. Preserve unsupported content visibly instead of misrepresenting it as plain assistant text. Keep transport-specific decoding in the adapter.

### Resolve queue and cancellation semantics explicitly

The prototype's `after_settled()` starts the next in-memory prompt and its Stop action retains queued work. Pi's `followUp()` already queues durable input; its documented `abort()` withdraws queued input as well as aborting active work.

For real mode, make Pi the sole queue executor and remove client-side automatic dequeue/submission from that path. Preserve the intended Stop behavior by adding an engine operation that stops active work while retaining and pausing the queue, plus explicit queue resumption. If the engine contract instead keeps “stop and clear queue,” label it exactly and deliberately revise the product behavior before exposing it. Never wire `abort()` to a button promising queue preservation. Test completion/stop/enqueue/remove races and reconnect with pending entries.

Steering and queue additions need independent request IDs even when attached to one run. A stale acceptance must not mark another prompt delivered. Do not silently discard attachments from queued or steering input as the current text-only queue path does.

## 5. Establish the engine contract and Rust client

Pin the engine and document a bounded desktop service contract. Application capabilities need their own versions; matching the envelope version alone is insufficient.

Implement in dependency order:

1. Engine build/run artifact with server, coordinator, workers, runtime dependencies, and required assets included. Publish a machine-readable version/capability manifest and readiness result. Source-only `PI_EXPERIMENTAL=1` scripts are a development entry point, not the shipping launcher.
2. Authenticated local transport, restrictive endpoint permissions, expected `serverId` validation, and explicit profile discovery. A random attachment ID or matching server ID is not authentication. Start with Unix sockets; implement equivalent access control before other platforms ship.
3. Length-prefixed CBOR framing and strict-JSON validation using Pi's fixtures, including fragmentation, coalescing, bounds, unknown fields, numeric limits, and malformed input. Negotiate compatible limits; return actionable mismatch errors.
4. The required Chord service catalogue/control calls, method invocation, errors, subscription lifecycle, initial snapshots, sequencing, and Delta path dictionaries/codecs. Hydrate before releasing buffered updates. Missing revisions invalidate the replica and trigger reattachment/resnapshot.
5. Typed bindings for SessionDirectory, SessionManagement, Models, AgentController, Transcript, and the new desktop-required history/metadata/status/auth capabilities. Missing optional capabilities disable the affected action with a reason.
6. Reconnect using a fresh authenticated connection and attachment generation; restore subscriptions and reconcile requests. Dispose retired subscriptions and bound retries. No automatic mutation replay.

The Rust conformance suite must exchange generated valid/invalid fixtures with Pi's TypeScript implementation and then attach to a real test server. A successful hello frame is not a completed integration milestone.

Backend deliverables owned in the Pi repository:

- Project cwd on session creation, canonical session metadata, title/rename, updated activity, and documented missing/moved project behavior. Existing session metadata on disk does not make it available through the service DTOs.
- Durable client request deduplication and lookup for prompt/steer/follow-up and other retryable mutations; operation and queue status sufficient for reconnect.
- Authoritative history paging, stable identity/cursor semantics through compaction and live appends, and access to complete stored tool results where available.
- Stop/pause/resume queue semantics from Section 4, plus an authoritative settlement signal.
- Provider authentication/status and the initial native extension interaction contract.

Implement these as real service changes, with TypeScript contract tests and matching Rust fixtures. Do not fabricate status by searching prompt text or inspecting private SQLite tables from Pipkin.

## 6. Make submission and recovery crash-safe

The critical trace is: **submit → engine accepts → connection or UI dies before acknowledgment**. The prototype's unknown state is useful presentation, but its operation IDs restart at 1 and its pending submission is not durable.

Required submission sequence:

1. Assign a globally unique client request ID scoped to an engine profile and session. Capture text, attachment content/reference semantics, model configuration, and mutation kind.
2. Transactionally persist that intent before clearing the recoverable draft or sending anything. If persistence fails, keep the draft and disable dispatch with a visible reason.
3. Engine acceptance atomically records the deduplication key with the durable submission. Reusing the same key/payload returns the same operation; reusing it with different content fails. Define retention and expired-key behavior.
4. Record the engine operation/entry ID and reconcile the optimistic row. Keep “accepted” distinct from “completed.”
5. On timeout/disconnect/restart, look up the same request. Unknown stays unknown; “not found” only means safe to resubmit when the server can prove non-acceptance within the deduplication contract. A lost receipt or expired record is not that proof.
6. Mark settlement durably and retain enough reconciliation state to prevent duplicate acceptance across desktop or engine restarts.

Request deduplication does not guarantee exactly-once shell/network/file effects. A tool can partially execute before failure. Explain partial completion and require inspection before an explicit new run. Separate a rejected submission retry from restarting a failed run that may already have changed files.

| Failure | Required result |
| --- | --- |
| UI crash before send | Journal/draft recovers; no unrecorded dispatch |
| UI crash or connection loss after acceptance | Reattach and recover the same operation without duplicate prompt/tool execution |
| Engine crash | Bounded restart, inspect durable runtime state, show recovered/interrupted status; no blind prompt replay |
| Stale or missing replica update | Reject stale route/generation, resnapshot after gaps, preserve reading position where IDs survive |
| Cancellation delay or disconnect | Remain stopping/unknown until authoritative settlement; transport request cancellation is not agent cancellation |
| Provider retry/auth failure | Show engine retry/error state; client does not invent its own duplicate run retry |
| Storage full/corrupt/incompatible | Explicit unsaved/recovery mode, preserve memory text, offer export; do not claim a temporary fallback database is durable user storage |
| Repeated host crash | Stop restart loop, surface diagnostics and recovery actions |

## 7. Complete durable desktop state and engine lifecycle

Upgrade storage with tested migrations and backups. Keep demo history out of engine storage and namespace **all** session-scoped desktop data by backend/profile/server/session. The existing `demo_conversations` table alone does not isolate numeric draft IDs and selection preferences.

Persist draft text and attachments together; track add/remove changes in revisions and restore missing-file validation on reopen. Include selected model intent only where it cannot conflict with authoritative session settings. Persist request intents and recoverable UI state. Queue contents come from Pi; offline unsubmitted drafts must never masquerade as accepted queue entries.

Keep the single writer and commit-acknowledged save state. Surface preference and metadata write failures, startup read failures, and fallback mode. Support retry/export from failed-save state; flush latest dirty revisions on switching and orderly exit without blocking rendering. Preserve unsent input on rejected commands even if a newer draft already exists. Refuse incompatible newer schemas; coordinate application rollback with database compatibility. Test process-crash durability separately from host/power failure.

Engine lifecycle policy for the first release:

- Use one managed local engine profile with worker ownership retained by Pi. Closing the desktop detaches the UI and flushes desktop state; active engine work can survive and be reattached on relaunch.
- Provide a distinct quit/stop-engine action with active-work disclosure. Shut down only processes owned by this profile; never kill an unrelated installed Pi instance.
- Coordinate launch/attach across multiple windows or app launches with a lock and verified endpoint. Do not spawn duplicate engines or sessions after readiness timeouts.
- Launch with explicit executable, argument array, cwd, and environment. Do not depend on interactive shell startup files. Detect missing tools and invalid cwd before dispatch.
- Keep stdout/stderr bounded and redacted, readiness timed, shutdown orderly, crash restart capped. Exercise desktop-icon launch, sleep/resume, stale sockets, and orphaned worker recovery.
- Bundle the tested engine for release. External development engine overrides are explicit and version checked; general independently installed engine support can follow later.

Read-only offline history comes from a rebuildable cache marked with last synchronization time. Draft editing remains available offline; executing/queueing requires a ready, reconciled connection.

## 8. Connect every existing surface to actual work

### Projects, conversations, history, and search

Use a native folder picker and real cwd validation. Support non-Git projects and moved/deleted directories. Add engine-backed create/rename and recover their outcomes; do not retain `SaveConversation` as the authoritative real-session mutation.

Open existing Pi sessions and hydrate active operations as well as transcript content. Preserve draft, scroll anchor, selection, and tool expansion when switching. Initial alpha may attach one session at a time; engine work continues while detached. Add separate connections/subscriptions later only when background live observation is implemented and bounded.

History paging must reach entries before compaction, avoid duplicates at the live/history boundary, and preserve stable IDs. Keep mounted views and cached pages bounded through repeated traversal. Search project/conversation metadata first, then indexed historical messages; clearly disclose partial/offline coverage. Opening a result loads its context and offers a predictable return path.

### Models, credentials, and attachments

Populate the model menu from Pi's catalog using `(provider, modelId)`, including model/thinking configuration and credential readiness. Serialize selection with prompt dispatch or extend the engine request contract so a send uses the intended configuration. Never display a local model preference as confirmed engine state.

Reuse Pi's established credential mechanism initially; add native setup/status flows without copying secrets into drafts/preferences or logs. Missing credentials, expired login, no models, and provider failures need actionable states. Routine tests use fake providers, not credentials or paid requests.

The inspected prompt contract accepts message text and optional encoded images; it does **not** accept arbitrary filesystem attachments. Define each attachment type: project file reference rendered into the prompt, bounded text content inclusion, or validated image content. Preview what is sent, enforce type/size limits, revalidate files at send time, preserve payload semantics for reconciliation, and report inaccessible/changed files. Add transport/blob support if larger payloads require it; do not silently truncate or pretend a selected file was delivered.

### Tools and workspace changes

Render actual tool identity, arguments, progress, output, errors, and settlement from engine state. Keep the existing bounded preview but provide retrieval/export of complete results when supported. Treat streamed output and Markdown as untrusted content; no automatic remote image fetches or command execution from rendered text.

Replace fixture diffs with an asynchronous read-only workspace service. Inspect staged, unstaged, untracked, renamed/deleted, and binary files; bound large patches and handle non-Git/missing repositories. Refresh on tool settlement and debounced filesystem changes, with explicit refresh/error/stale states. Use argument arrays and validated paths for Git/editor/terminal calls.

Keep the label **Workspace changes**: these can include user edits and unrelated work. Do not claim run attribution or offer “Undo run” without snapshots and conflict handling. Patch application/reversion is deferred; actual Pi tools may still edit project files as part of requested work.

### Extensions, commands, and interaction quality

Keep extension execution in Pi. Define native selection, confirmation, input/editor prompts, notifications, and status interactions with stable request IDs, cancellation, timeouts, and disconnect behavior. Pi's current presentation-local slash commands and TUI facets are not automatically remote desktop capabilities. Publish a compatibility matrix; unsupported terminal-only interactions fail visibly instead of hanging a run. Do not load arbitrary extension code into GPUI.

Continue one command/availability registry for buttons, menus, shortcuts, and palette. Add real project/auth/reconnect actions, configurable keybindings and send behavior, external editor/terminal launching, and release-appropriate settings. Keep demo labeling and developer scenario actions only in explicit demo mode; real mode must never fall back to simulated success.

## 9. Close prototype gates while integration proceeds

Use [scorecard.md](scorecard.md) as the baseline of measured, observed, failed, and unverified results. Do not turn an unverified entry into a pass because integration builds.

- Correct composer caret/IME candidate geometry, including `invalidate_character_coordinates()` where required; test actual compose/commit/cancel and no premature Enter-send with a configured IME.
- Expose composer text, caret, and selection to accessibility; complete an actual screen-reader workflow through navigation, composer, transcript, tools, and run status. An inspected AT-SPI tree is insufficient.
- Verify both themes, enlarged text, visible focus, reduced motion, narrow layouts, 125%/150% scaling, minimize/restore, and suspend/resume on the actual compositor.
- Measure display presentation separately from frame submission, usable startup separately from window mapping, true cold startup separately from partially evicted caches, and desktop memory separately from engine/worker memory.
- Re-test offscreen selection, history prepend, tool expansion, resize anchoring, and repeated traversal under real service updates. Coalesce expensive Markdown/highlight work, cancel obsolete parses, and bound caches without breaking selection.
- Review the pinned dependency patches noted in [decisions.md](decisions.md) and the GPUI skill. Document which are necessary for the selected revision and validate any adopted patch; avoid unmeasured dependency churn.

The initial targets remain warm usable UI ≤1 s, cold ≤2.5 s, p95 input-to-presentation <50 ms, p95 scrolling within 16.7 ms at 60 Hz, and desktop idle RSS <250 MB on the recorded reference fixture. Provider readiness must not block initial usable UI. Record sample sizes, workloads, engine resource budgets, and any revised targets before judging the result.

## 10. Delivery sequence and acceptance criteria

Each milestone produces a runnable build, targeted tests, and an updated integration scorecard. Dependencies determine order; dates should be estimated after M1 exposes the protocol and backend work. The old six-to-nine-month estimate assumed a different staffing/platform scope and is not a new commitment.

| Milestone | Work and primary ownership | Exit criterion |
| --- | --- | --- |
| **M0 — Preserve and prepare** | Pipkin: explicit real/demo composition, namespaced storage migration, asynchronous backend lifecycle, source/version manifest, retained demo regression suite | Existing demo remains deterministic; real mode starts with honest connecting/unavailable states and cannot show fixtures as real data |
| **M1 — Real read path** | Pi: runnable pinned host, auth and metadata contract. Rust: protocol/Chord conformance, attach, Models/Transcript hydration | From a clean developer setup, authenticate to a real Pi test server, list/open a session, render its actual state, switch and reject delayed old-attachment updates; no live provider required |
| **M2 — First complete real workflow** | Pi + Pipkin: project/session creation, model selection, prompt/tool streaming, journal/dedup/status, engine lifecycle, actual read-only diffs | In a temporary project, a real Pi engine with a deterministic test provider executes a file-changing tool; Pipkin displays the output/diff and reopens the same history. Drop acknowledgment and kill/relaunch UI without accepting the prompt twice |
| **M3 — Daily-use execution and recovery** | Pi + Pipkin: authoritative steering/queue/stop, reconnect/status settlement, credentials, attachments, persistence failures | Steer, queue/remove, stop, resume, switch, and restart work correctly; attachments survive and reach the intended input path; engine/UI/disconnect/storage fault matrix passes |
| **M4 — Complete local product** | Pipkin + required Pi services: pre-compaction history, search, complete tool results, supported extension dialogs, editor/terminal actions, offline cache; close native gates | The core workflow is usable without demo data; 10,000-message traversal remains bounded; real IME/screen reader/scaling/lifecycle checks pass; unsupported features are explicit |
| **M5 — Distributable Wayland alpha** | Packaging/platform: bundled engine/runtime, desktop integration, clean install, upgrades/rollback, diagnostics, soak tests | A clean supported Omarchy/Arch installation runs the workflow from a desktop launcher without adjacent source checkouts; upgrade and recovery preserve acknowledged drafts/history |
| **M6 — Broader release** | Platform qualification and beta | Each advertised OS passes installation, text/accessibility, graphics, process/storage recovery, and update tests; measured beta reliability and usability targets are met |

M2 is the first genuinely functioning client, not the end of the product work. M3–M5 are required before calling it a reliable daily-use local application. Native accessibility/platform work begins at M0 and remains a release gate, rather than being postponed to M6.

### First implementation batch

1. Capture current demo checks and record the two repository revisions; update the continuation map to the verified contract inventory when implementation begins.
2. Add explicit backend selection and lifecycle/capability state, keep the current demo path, and remove hard-coded demo dependencies from real-mode composition.
3. Design and migrate stable session namespaces, full drafts, and request intents before any live mutation path is enabled.
4. Build the engine test launcher and Rust framing/Chord fixture harness; implement authenticated attach and a real read-only transcript/model path.
5. Land Pi project metadata and idempotent submission/status contracts with tests, then connect submit and recovery.
6. Demonstrate one real tool edit, actual diff, reconnect, and relaunch in a disposable project, using a fake provider. Keep live-provider verification separate and explicitly controlled.

### Implementation status (updated 2026-10-04; read this first when resuming)

**Where we are: M0 is done. M1 is met against a real Pi server, with the caveats below.** Everything through the M1 code is committed and pushed; the later real-server work (attach retry, third real test, this doc) is not yet committed.

| Milestone | State | Evidence |
| --- | --- | --- |
| M0 preserve and prepare | **Done** (open: source/version manifest) | Explicit `Mode`, `Connection` state, async `Backend::start(sink)`, namespaced storage v3, crash-safe submit journal v4 |
| M1 real read path | **Met, with caveats** | Real handshake, trust checks, catalogue, Delta hydration, session listing, attach, `Transcript` subscription and switching all pass against a real Pi server (3 opt-in tests, run repeatedly). The app was launched against it and showed the real session list and an opened session |
| M2 and later | Not started | |

`cargo test --workspace`: 297 pass, 3 ignored (the real-server tests). Clippy and fmt clean.

#### M1 caveats (what the real-server runs did NOT cover)

- **Only empty transcripts were seen from the real server.** A new session has no messages, so the `ConversationView` mapper has never met real entries. Its entry kinds, message shapes and `pi.live` slots are inferred from Pi's source. A non-empty real transcript needs a deterministic provider: Pi has `packages/ai/src/providers/faux.ts`, but it is an in-process test library, so using it means scripting Pi's own test harness (see `coding-agent/test/experimental-agent-controller.test.ts` and `experimental-durable-support.ts`). That is M2-sized and was not started.
- **Delayed-frame rejection is proven at the client and mock level**, not by injecting frames into the real server. On the real server we verified what a switch does: a fresh attachment id, the old subscription retired locally, and calls to the old route refused locally.
- **GUI check was limited and passive** (a window screenshot, no input): session list, derived title and age, an opened empty session, no demo marker. Switching in the GUI, and the offline/failed/incompatible screens, were not looked at.
- Pi's `SessionSummary.createdAt` is in **milliseconds** (handled); the title time is shown in UTC.

#### Findings from first contact with the real server

1. **Pi server bug (report upstream):** re-attaching a session right after switching away from it fails with `internal_error: Internal server error`. Reproducible and deterministic with no pause (open A, open B, open A); with 300 ms or more between switches it never fails, so the session's worker is still retiring. The server swallows the real exception (no error sink is wired; nothing reaches stderr), so the cause is unconfirmed beyond that timing. **Pipkin's mitigation:** `PiBackend` retries `attach` on `internal_error` only, at 150/400/1000/2000 ms, logs each retry and then gives up with a visible error; other error codes are final. Attach only navigates, so the retry is safe. Covered by two mock tests and `real_server_switching_between_sessions`.
2. The server-scope catalogue lists only `pi.session-directory`, `pi.session-management`, `pi.presentation-plugins`; `pi.models`, `pi.transcript`, `pi.agent-controller` appear after attach (session scope), as designed.
3. **UI defects seen in real mode (not yet fixed):** the "New conversation" button is shown enabled but does nothing (the core refuses local creation in real mode); the empty state says "No messages yet. Write a prompt below to begin" although prompts are refused; "Choose model" is empty because the test server has no providers. Each visible control should either work or show a clear unavailable state.

#### What exists

- **`crates/pi-client`** (no GPUI): strict CBOR subset, framing, protocol v8 envelopes, Chord wire grammar and per-subscription Delta codecs, replica with sequence-gap detection, the connection state machine (handshake, request correlation and cancel, hydration buffering, **stale-attachment fencing**, no reconnect or replay), Unix transport with trust checks (private dir and socket owned by us, no symlinks, `SO_PEERCRED` uid, `serverId` handshake, discovery that reports untrusted sockets), and `testing.rs`, a mock Pi server that speaks the real wire.
- **`crates/pipkin-app/src/adapters/pi/`**: `PiBackend` (worker thread: discover, connect, mirror the session directory into the catalog, open a session = attach + subscribe `pi.transcript` and `pi.models`, `Synced` live updates, reconnect with backoff that refreshes the open conversation, bounded attach retry), `transcript.rs` (pure `ConversationView` mapper; never panics; unknown kinds shown as notices), `session.rs` (stable conversation ids, derived titles, model catalogue), `real_pi.rs` (opt-in real-server tests).
- **Core**: `Mode`, `Connection`, `LifecycleEvent`, `RequestId` and the submit journal, `EventKind::{Synced, OpenFailed}`, `preview_output`. **Storage**: schema v4, backup before migration.
- **`scripts/pi-test-server.sh`**: throwaway Pi experimental server with isolated dirs, `PI_OFFLINE=1`, no credentials.
- Real mode is the default; `--pi-dir`, `--pi-server-id` choose the server. `--demo <scenario>` keeps the simulator for regression tests only. This build is **read-only**: prompts are refused with "Sending prompts is not available yet".

#### How to run the real-server checks

Needs the Pi checkout's dependencies (`npm ci` in `../pi`) and its generated model catalog (`npm run generate-models` in `../pi/packages/ai`, which makes unauthenticated GETs to public catalogs; both were done on this machine). Unix socket paths are limited to about 108 bytes, so keep the root short (for example `/tmp/pipkin-pi`, not a long scratch path).

```
scripts/pi-test-server.sh /tmp/pipkin-pi          # terminal 1, foreground
eval "$(scripts/pi-test-server.sh /tmp/pipkin-pi --print-env)"
cargo test -p pipkin-app real_pi -- --ignored --nocapture     # terminal 2
cargo run -p pipkin-app --release -- --pi-dir /tmp/pipkin-pi/server \
  --pi-server-id 5f0c7b1e-2d4a-4f6b-9a3e-1c8d7e6f5a40 --data-dir /tmp/pipkin-app
```

`PIPKIN_REAL_PI_SETTLE_MS=<ms>` adds a pause between switches in the switching test, to probe the server race.

#### Housekeeping

- `AGENTS.md` still references `scripts/guard.sh`, which does not exist. Native input testing was not done; do not send synthetic input without recreating a guard.
- `docs/baseline.md` carries the old demo scorecard figures; they are historical.

#### Next work, in order

1. **Commit and push** the uncommitted real-server work.
2. **Finish M1 properly:** get a non-empty real transcript through a faux-provider harness and verify the mapper against real entries; fix the three real-mode UI defects above; look at switching and the offline screens in the GUI; report the attach race to Pi.
3. **M0 leftovers:** build manifest (tested Pi revision and service-contract version); surface storage fallback and load failures in the UI; restore from backup.
4. **M2 (first complete real workflow):** `AgentController` binding and idempotent submit (needs Pi-side client request ids and lookup, section 5), run/queue state from the engine, model selection via `Models.select`, project cwd and session metadata (Pi-side), real workspace diffs, engine lifecycle (launch and own a pinned engine), journal recovery for real sessions, journaling of steer and follow-up requests.
5. Smaller gaps: `Synced` replaces all items on each change (coalesce and reconcile by stable id in M2); no `has_older` paging; keyed-service replica support is untested against a real server.

## 11. Verification and release gates

Retain all seven demo scenarios, but add integration tests using the actual Pi server/worker/runtime. Simulation alone cannot pass an integration gate.

| Test layer | Required evidence |
| --- | --- |
| Core | Availability with connection/capability state; stable identities; exact prompt/steer acknowledgment mapping; queue/stop races; stale events and snapshot reconciliation |
| Protocol | Rust↔TypeScript fixtures, service catalogues, hydration buffering, Delta dictionaries, gap detection, malformed/oversized frames, disconnect during request, route fencing, authentication rejection |
| Engine integration | Real sessions/cwd/model selection, fake provider streaming, actual temporary-file tools, historical pages, queue consumption/removal, settlement and extension responses |
| Crash/recovery | Kill UI/engine before journal commit, before/after engine acceptance, before acknowledgment, during tool execution, while stopping, during page hydration, and on upgrade; assert no duplicate acceptance or lost acknowledged draft |
| Storage | Full draft/attachment round trips, demo/real isolation, queue saturation, failed-save retry, disk full, corruption, newer schema, interrupted migration, backup restore, rollback compatibility |
| UI/native | Actual keyboard, clipboard, IME, accessibility, focus restoration, themes/large text, scaling, selection, scroll anchoring, narrow layouts, sleep/resume and graphics |
| Soak/performance | Long streams, huge tool output/diffs, repeated 10,000-message traversal, session switches/reconnects; measure bounded caches, queue depths, retained subscriptions, CPU/RSS and frame latency |
| Distribution | Clean launch without development checkout, engine mismatch, missing credentials/tools, installation/update interruption, executable discovery/environment, uninstallation that preserves user data |

For Pipkin changes, run `rtk cargo test --workspace`, `rtk cargo clippy --workspace --all-targets`, and `rtk cargo fmt --all --check`. For Pi changes, follow that repository's required checks and run targeted contract/runtime tests with fake providers. Record commands and outcomes. Native input automation must follow Pipkin's guard script and stay confined to the test window; system configuration/package changes are separate explicit actions.

Release blockers: acknowledged data loss, duplicate accepted mutations, wrong-project execution, silent simulated fallback, unresolved stop state, unauthenticated local control, essential accessibility failures, and nonrecoverable upgrade failures. Maintain bounded crash retries and actionable diagnostics. Collect no prompts, code, tool output, or credentials by default.

Preserve the original beta targets of 99.9% crash-free launches and 90% unassisted core-task completion, but define population/sample size and report observed evidence before claiming either. A small successful developer run cannot establish those rates.

## 12. Platform and product completion

| Release track | Required coverage before advertising support |
| --- | --- |
| Omarchy/Arch Wayland alpha | Hyprland tiling, portals, clipboard, missing keyring, actual scale/IME/screen reader, Intel/AMD/NVIDIA coverage appropriate to the support claim; maintained Arch packaging and bundled engine |
| Other Linux | GNOME/KDE Wayland, declared runtime/library baseline, practical distribution artifact; X11 feature/build and native qualification (currently disabled in the pinned manifest) |
| macOS | Platform initialization/features, Apple Silicon and explicit Intel policy, menus/shortcuts, Keychain, VoiceOver, process lifecycle, signed/notarized package |
| Windows | Platform initialization/features, authenticated local transport, Unicode/long paths, process tree ownership, per-monitor DPI, screen reader, signed installer; WSL remains a separate future capability |

Across tracks, support secure credential storage or the engine's established mechanism, explicit environment/tool discovery, drag-and-drop and folder/editor/terminal actions, and a signed/verifiable update path with a bootable prior version. Do not claim isolation: Pi tools execute with the engine process's permissions unless an actual execution boundary is implemented.

The roadmap is complete when the selected release track can install and run the entire real workflow, every visible control has authoritative behavior or a clear unavailable state, recovery is demonstrated at the acknowledgment and process boundaries, and the native interaction gates pass. Preserve the demo for regression work and the existing visual foundation throughout.
