# State core, effects, backend port, persistence

## Shape
`Command` (UI intent) → `AppState::dispatch` → `Outcome { effects, notes }`. `BackendEvent` → `AppState::apply_event` → `Outcome`. The GPUI `Model` entity (`desktop-ui/src/model.rs`) owns `AppState`, runs effects through an installed `EffectHandler` (installed by the controller in `desktop-app`), `cx.emit(Note)` and `cx.notify()`. Views subscribe to `Note` to splice lists instead of rebuilding.

- `Effect`: `Backend(BackendRequest)`, `SaveDraft{conv,text,rev}`, `SavePrefs`, `SaveConversation`.
- `Note`: `ItemsReset|ItemsPrepended(n)|ItemsAppended(n)|ItemChanged(i)|ConversationsChanged|SelectionChanged|Other`. The transcript view maps these to `ListState::{reset,splice,remeasure_items}`.
- `Backend` trait is only `bootstrap()` + `request(BackendRequest)`; events return on an `async_channel` handed to the adapter. Slow, lost and late responses are representable. Do not invent the real wire protocol in the port.

## Correctness rules encoded in the core (all unit-tested in `desktop-core/tests/transitions.rs`)
- **Stale guard:** every event carries `(conversation, generation, op)`. Drop if the generation differs from the conversation's, or if an op-scoped event's op is not the live op. A late event for conversation A must land in A even when B is selected.
- **Run states:** `Idle → Submitting → Running → Stopping → Idle`, plus `OutcomeUnknown` and `Failed`. `Cancel` moves to `Stopping` and stays until the backend sends `Cancelled`. Cancel is not available again while Stopping.
- **Unknown outcome:** `AckLost` during `Submitting` → `OutcomeUnknown`. Submit/Retry are unavailable. Only `CheckStatus` is, and the backend's `StatusResolved` decides. Never auto-resend. The demo only proves client presentation, not engine exactly-once.
- **Rejected submit:** mark the user item `Rejected`, restore the text to the draft if the editor is empty, bump `sync_epoch`, set `Failed`; Retry is an explicit command.
- **Queue:** queued prompts auto-start after `Completed`, not after a user `Cancel`.
- **Draft/save:** per-conversation draft with `rev`, `sync_epoch` (core-originated text changes: the editor resyncs when it changes), and `SaveState {Clean,Dirty,Saving,Saved,Failed}`. "Saved" only after the writer acknowledges a commit **and** `rev` still matches; an edit during the write leaves it `Dirty`. Flush on conversation switch and orderly exit. On failure keep the text in memory and offer Copy draft.
- **Tool output bound:** keep a bounded preview (8 KiB, cut at a char boundary), `truncated` + `full_len`.
- **Availability** is one function (`AppState::availability`) used by buttons, shortcuts and the palette so they cannot disagree.

## Syncing the editor with the core
The composer owns live text. `ComposerEvent::Changed` → `EditDraft(text)`. When the core changes the draft (submit clears it, rejection restores it, conversation switch) it bumps `sync_epoch`; the workspace's `observe_in(&model)` callback calls `composer.set_text(..)` (which must NOT emit `Changed`, or you loop). Do this in the observer, not in `render`.

## Controller (desktop-app)
- `start(cx, options) -> Entity<Model>`: open storage, build demo backend, build `AppState::new(bootstrap, prefs)`, apply stored conversations/drafts **before** creating the entity, install the effect handler, spawn a foreground task that drains the event channel in batches (coalescing consecutive tokens of one op), set `DemoControls` global, register `cx.on_app_quit` to flush dirty drafts and `storage.shutdown()`.
- Real clock only through `AppState::set_now` on a timer; the demo base time is fixed for determinism.

## SQLite storage (`storage.rs`)
Versioned (`PRAGMA user_version` migrations, refuse newer), WAL, `synchronous=FULL`, **one** writer thread on a bounded queue (256; full queue returns an error instead of blocking the UI). Acks fire only after the transaction commits. Demo-owned data lives in `demo_*` tables so a real adapter can replace history without migration. Injected write failure is an `AtomicBool`. The kill test spawns the test binary itself, reads `ACK` lines, SIGKILLs after N acks, reopens and asserts every acked draft is intact. Same behavior was confirmed natively with `kill -9`.

## Deterministic demo backend
Own xorshift PRNG (no rand), fixed base timestamp, scenario scripts as versioned JSON (`"version": 1`, reject unknown) embedded with `include_str!`, per-conversation attempt counters (failure scenario cycles), cancel settles after a delay (not instantly), steer is remembered until the script's next steer point. Scenarios: normal, followup, failure, unknown, stressed, large, persist-fail. Generated 10,000-message history with stable ids, pages of 200. Arbitrary user text is acknowledged as scripted and never acted on; executable-looking commands are inert text. Headless tests drive `AppState` + `DemoBackend` at `speed=0` through every scenario and compare `Debug` output across seeds for determinism.
