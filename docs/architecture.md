# Architecture

Three crates, one direction of dependency: `desktop-app → desktop-ui → desktop-core`.

```text
GPUI views ──Command──▶ Model entity ──▶ AppState (desktop-core, pure Rust)
                              │                 │
                              │◀── Effects ─────┘
                              ▼
                 controller (desktop-app) executes effects
                    ├─ Backend port: DemoBackend now, Pi adapter later
                    └─ Storage: SQLite, single writer thread
        BackendEvent ◀── backend ──▶ Model::apply_event (stale-guarded)
```

- **desktop-core** owns identities (project, conversation, item, operation, queue), run-state transitions, queue contents, command availability (`AppState::availability`) and draft/save state. No GPUI, colors, geometry or transport frames. 13 transition tests cover stale events, cancellation, unknown outcome, queue, drafts.
- **desktop-ui** owns focus, selection geometry, text layout, scroll handles and open overlays: `theme` (semantic tokens), `text` (`ComposerEditor`), `transcript` (`TranscriptDocument`, `TranscriptView`), `shell` (workspace, navigation, inspector, overlays, command registry).
- **desktop-app** owns the entry point, controller, `adapters/demo.rs`, `storage.rs`, `platform.rs`.

Stale-update guards: every `BackendEvent` carries `(conversation, generation, op)`. `AppState::apply_event` routes by conversation id, drops events from an older attachment generation, and drops operation-scoped events whose op is not the live one. A late event never lands in whichever conversation is selected.

Execution states (`RunState`) are separate from persistence (`SaveState`) and, later, connection state. Cancel stays `Stopping` until the backend confirms. `OutcomeUnknown` is never resent automatically; the only exits are `CheckStatus` (backend answers) or a failed resolution.

Render functions do no I/O. Draft saves are debounced (500 ms) and flushed on conversation change and orderly exit; "Draft saved" is shown only after the writer acknowledges a commit.
