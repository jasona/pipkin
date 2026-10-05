# Beta: targets and how they are measured

There is no telemetry. Pipkin sends nothing anywhere about its use. Beta evidence comes from people running it
and reporting, with `pipkin --diagnose --probe` output (home directory hidden, no credentials) attached to a
report. Everything below is a target to measure against; **none has been measured**, because there has been no
beta.

## Reliability targets

| Target | How to measure | Where the tooling stands |
| --- | --- | --- |
| No acknowledged draft or sent prompt is lost across a crash, kill or upgrade | Crash/recovery suite (kill before and after the journal commit, during a run, on upgrade); upgrade test from every earlier schema | Built and passing in `cargo test` and the real-engine suite |
| No prompt reaches the model twice after a lost acknowledgment or reconnect | Real-engine fault-proxy tests | Built and passing |
| Memory and open files stay flat over long use | `many_prompts_in_one_conversation_do_not_grow_memory_or_open_files` with a large `PIPKIN_SOAK_ROUNDS`; a day-long session | 1500 prompts measured; the day-long run is owner-run |
| The engine starts, or says clearly why not | `--diagnose --probe`; the in-window strip when the engine is unavailable | Built; the strip is checked by build only |
| An older Pipkin never damages a newer database | Newer-schema refusal test, backups at each upgrade | Built and passing |

## Usability targets

| Target | How to measure | Status |
| --- | --- | --- |
| A new user reaches a first answer from a model without reading docs | Watch 5 people from install to first reply; count stalls | Not measured |
| Every visible control either works or says why it is unavailable | Walk the UI at 125% and 150%, both themes, with the keyboard only | Partly walked (see `native-gate-testing.md`) |
| Keyboard-only use and a screen reader can run the whole workflow | Orca and keyboard pass | Orca partly working; not passed |

## Gates a beta must pass before wider release

The owner-run gates in `native-gate-testing.md`, the install gates on a clean machine, and the platforms
claimed in `platforms.md`.
