# Continuation map

Next step if approved: obtain and verify the real Pi service sources (absent from this workspace), then add a Pi adapter beside `adapters/demo.rs`.

## Backend port (`pipkin_core::Backend` + `BackendRequest`/`BackendEvent`)
| Application capability | Adapter work | Backend facts still needed |
| --- | --- | --- |
| `bootstrap()` projects, models, conversation summaries | Map engine session/project listing and model discovery | Project identity, model catalog source |
| `Open` / `LoadOlder` | Page authoritative history into `TranscriptItem`s | Paging cursor semantics, item ids stable across restarts |
| `Submit` (+ `Accepted`/`Rejected`/`AckLost`) | Send prompt with attachments and model | Idempotency key, acknowledgment shape |
| `CheckStatus` | Look up an operation by request id after reconnect | Whether the engine can answer by request id (exactly-once is **not** proven by the demo) |
| `Steer`, `Cancel` (+ `Cancelled`) | Mid-run steering, cancel with settlement | Cancel settlement signal |
| Streaming (`Token`, `Tool*`, `ChangesReported`, `Completed`, `Failed`) | Translate engine frames; coalesce deltas | Frame types, tool output size limits, diff source (replaces fixture diffs) |

Not invented here: RPC/Chord/CBOR framing, auth, process management, version negotiation. Those belong in the adapter once the contract is verified.

## Keep as is
UI, command registry, component system, local draft storage (`Storage`, distinct `demo_` namespace), and deterministic scenario tests. Demo history must not migrate into engine storage.
