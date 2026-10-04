# Demo scenarios

The simulated Pi backend (`crates/desktop-app/src/adapters/demo.rs`) answers the **next Submit** according to the
selected scenario. Seeded projects and conversations exist regardless of scenario. Scripts live in
`fixtures/scenarios/<name>.json` (`"version": 1`; unknown versions are rejected) and are embedded at build time.

## Running

```sh
cargo run -p desktop-app --release -- --demo normal
cargo run -p desktop-app --release -- --demo followup --data-dir /tmp/pi-demo --speed 2
```

| Flag | Meaning |
| --- | --- |
| `--demo <scenario>` | Initial scenario (default `normal`). Can be changed later from the Developer commands. |
| `--data-dir <path>` | Where `pi-desktop.sqlite3` lives. Also `PI_DESKTOP_DATA`; default `$XDG_DATA_HOME/pi-desktop`, else `~/.local/share/pi-desktop`. |
| `--speed <factor>` | `1.0` real time (default), `2` twice as fast, `0` never sleeps. At `0`, steer points pass through. |

## Scenarios

| Scenario | How to exercise it | What it shows |
| --- | --- | --- |
| `normal` | Send any text. | Accepted, streamed Markdown (chunks of 1-6 words), three `read_file` rows, an `edit`, a failing `bash` test run, a corrected edit plus a new test, a passing run, three changed files with real unified diffs, a final summary. The reply states it is a scripted demo and that it did not read or act on your message; it echoes your text as an inert quote. |
| `followup` | Send text, then steer, queue and cancel while it runs (~20 s at speed 1). | Three steer points (about 4 s each). Typing text and choosing Steer is acknowledged (`SteerAccepted`), and the remaining replies visibly acknowledge it. Queue two prompts and they run in order after completion. Cancel keeps the run in Stopping for about 350 ms until `Cancelled`; queued prompts stay queued. |
| `failure` | Send, then Retry, then send again. | Attempt 1: provider rejection (`Rejected`, draft retained, Retry offered). Attempt 2 (Retry): the normal story. Attempt 3: a tool fails with multi-line expandable output, then `Failed`. Attempt 4 cycles back to a rejection. The attempt counter is per conversation. |
| `unknown` | Send, then Check status. | `AckLost`: Outcome unknown, never resent automatically. Check status resolves the same operation (`StatusResolved{accepted:true}`) and the run continues as `normal`. The backend counts every Submit it receives; a repeated Submit is a new operation, never the original. |
| `stressed` | Send any text. | A reply with emoji, ZWJ sequences, combining marks, Arabic and Hebrew text, a very long unbroken token and path, an unterminated code fence, broken nested lists, an unclosed bold and a broken link. Command-looking text is inert. |
| `large` | Send any text. | A diff with more than 2,000 changed lines across five files, and a tool output of about 1.3 MB. The core keeps an 8 KB preview and the original length. |
| `persist-fail` | Send text; toggle the storage failure from Developer commands. | Replies exactly like `normal`. Turning on the storage failure makes draft saves return "injected write failure", so the draft shows as unsaved while the editor text stays intact. |

## Seeded conversations

Two projects: `pi-desktop` and `billing-service`. Twelve conversations, including:

- ordinary histories of 8 to 30 messages (prompts, Markdown with lists, links and fenced code in Rust, Python, TypeScript, SQL, Bash, Go, TOML and JSON, completed tool rows, one failed tool each), some with very long or Unicode titles;
- **Large history - 10,000 messages**: opens the latest 200 items; "load older" pages 200 at a time until all 10,000 are loaded (stable item ids, mixed heights, some tool rows with large output);
- **Large diff**: 5 files, over 2,000 changed lines in the inspector;
- **Huge tool output**: a tool row whose output was 1.5 MB (bounded preview, original length recorded);
- **Stress content**: long paths and titles, emoji, combining marks, ZWJ, RTL, malformed Markdown, a missing attachment;
- **Empty conversation**.

"Fix failing search test in desktop-core" also shows the three changed files from the `normal` story.

## Determinism

History, diffs, ids and timestamps are pure functions of the seed (xorshift, no wall clock, no `rand`). Two backends with the
same seed produce identical event sequences (tested by comparing `Debug` output). The desktop app shifts the fixture base time
to the start of the current day (UTC) so recency labels read naturally; tests use the fixed `DEFAULT_BASE_TIME`.

## Tests

```sh
cargo test -p desktop-app        # storage (incl. SIGKILL durability), scripts, headless scenario runs
cargo test -p desktop-core
```

The headless tests (`crates/desktop-app/src/adapters/tests.rs`) run `desktop_core::AppState` against `DemoBackend` at speed 0
with `hold_steer` enabled where a test must stop at a steer point. They need no GPUI window.
