# Beta: targets and how they are measured

Beta evidence comes from people running Pipkin and reporting; no application analytics collection is configured
by this client. Real model requests still send conversation/tool data to the chosen provider. For reports, prefer
**Ctrl K → Copy diagnostics** (limited metadata). Detailed `pipkin --diagnose --probe` output can contain sensitive
log/error text; home-directory substitution does **not** make it credential-free or safe to post. Review/redact it
and follow [support/privacy guidance](support.md) and the [private security route](../SECURITY.md). The owner now reports heavy daily-use beta testing, including building Pipkin itself and working on
other projects, and describes the app as being in great shape. This is **owner-reported workflow evidence**;
duration, incident rates, and new-user results are not quantified. The targets below remain targets rather
than measured beta rates. Current release qualification is tracked in [v1-release-gates.md](v1-release-gates.md).

## Reliability targets

| Target | How to measure | Where the tooling stands |
| --- | --- | --- |
| No acknowledged draft or sent prompt is lost across a crash, kill or upgrade | Crash/recovery suite (kill before and after the journal commit, during a run, on upgrade); upgrade test from every earlier schema | Built and passing in `cargo test` and the real-engine suite |
| No client-induced duplicate prompt dispatch after a lost acknowledgment or reconnect | Real-engine fault-proxy tests and durable input admission | Built and passing; not a universal exactly-once model/tool-effect guarantee |
| Memory and open files stay flat over long use | `many_prompts_in_one_conversation_do_not_grow_memory_or_open_files` with a large `PIPKIN_SOAK_ROUNDS`; a day-long session | Historical 1500-prompt measurement; final-candidate extended soak and owner-run day-long session remain open |
| The engine starts, or says clearly why not | `--diagnose --probe`; the in-window strip when the engine is unavailable | Automated unavailable/error paths covered; installed native walkthrough remains open |
| An older Pipkin never damages a newer database | Newer-schema refusal test, backups at each upgrade | Built and passing |

## Usability targets

| Target | How to measure | Status |
| --- | --- | --- |
| A new user reaches a first answer from a model without reading docs | Watch 5 people from install to first reply; count stalls | Not measured |
| Every visible control either works or says why it is unavailable | Walk the UI at 125% and 150%, both themes, with the keyboard only | Partly walked (see `native-gate-testing.md`) |
| Keyboard-only use and a screen reader can run the whole workflow | Orca and keyboard pass | Orca partly working; not passed |

## Gates a beta must pass before wider release

The owner-run gates in `native-gate-testing.md`, the install gates on a clean machine, and the platforms
claimed in `platforms.md`. Daily-use beta does not automatically close these gates; results must be recorded
for the relevant environment and release candidate. See the sequenced [v1 release plan](../llm-docs/pipkin-v1-release-plan.md).
