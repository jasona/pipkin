# Pipkin v1.0 release gates

## Working scope and baseline

Started **2026-10-06 UTC**, following the owner's request to execute
[`llm-docs/pipkin-v1-release-plan.md`](../llm-docs/pipkin-v1-release-plan.md).

Working scope: **x86_64 Arch/Omarchy, Hyprland, Wayland**. This is the plan's recommended initial scope, not a new claim of support for other distributions, compositors, or operating systems. Final platform/acceptance sign-off remains an owner decision.

Feature freeze: prioritize release defects, qualification, onboarding/support, and release machinery. Additional features belong in the post-v1 backlog unless required to close a blocker.

### Identified baseline

| Item | Value |
| --- | --- |
| Pipkin revision tested | `1e3d54c0e92b36de0aac97a8e77117238aecf582` |
| Pi engine revision tested | `d4c871ef75bea57f43518e5e554c5a4765173a0b` |
| Engine source state | Clean tracked working tree when baseline began; generated/ignored build inputs are not identified by this check |
| Application/package version | `0.0.1` |
| Database schema | 7 (`MIGRATIONS.len()` in `storage.rs`) |
| Rust | `rustc 1.99.0 (b940084d7 2026-09-28)` |
| Cargo | `cargo 1.99.0 (5f94df478 2026-08-27)` |
| Node | `v26.8.1` (qualification at the advertised minimum Node version remains open) |
| GPUI revision | `a84689073d296dfd39987bc7dd478e43ef76d83a` |
| Machine/display | Linux x86_64; Hyprland 0.56.2; reported active monitor 2560×1600 at scale 1.6 |

### Owner-reported beta evidence

The owner reports heavy daily-use beta testing, including using Pipkin to build itself and using it on other projects, and describes it as being in great shape. Duration, project count, crash/loss rates, and new-user onboarding results have not been quantified. This establishes useful owner-reported workflow evidence, not a measured general reliability rate or a clean-machine/platform pass.

### Automated baseline: passed on 2026-10-06

| Check | Result and boundary |
| --- | --- |
| `cargo test --workspace` | **505 passed, 36 ignored**, no failures. Ignored tests are not counted as passes. |
| `cargo clippy --workspace --all-targets` | Passed, no warnings reported. |
| `cargo fmt --all --check` | Passed. |
| Development-engine release e2e suite | **33 passed**, serial, using the identified adjacent engine checkout. Includes the default 40-prompt soak tripwire; not an extended/day-long soak. |
| `scripts/package.sh` | Built `dist/pipkin-0.0.1-x86_64.tar.zst` from this app/engine pair. |
| `scripts/verify-install.sh --full dist/pipkin-0.0.1-x86_64.tar.zst` | Scratch-prefix checks and **33 real-engine tests passed against the unpacked engine**. Empty HOME/minimal PATH; not a desktop launcher or clean-system GUI test. |
| `scripts/test-install.sh dist/pipkin-0.0.1-x86_64.tar.zst` | All 12 scratch-prefix checks passed: install, discovery/probe, simulated version upgrade, rollback, uninstall, and data-marker preservation. Not a real schema-changing app upgrade. |

Artifact SHA-256:

```text
7e426a7eb6eab92e38cf2f426c57e1ee6bde259d8ffec583438737fd610082c6  pipkin-0.0.1-x86_64.tar.zst
```

Measured size: tarball **89,114,048 bytes** (about 85 MiB); staged installation **453 MiB**; engine staging output **200 MiB**. These are current baseline measurements, not guaranteed final release sizes. The build is not signed and is not a release candidate.

Full command output was captured locally in `/tmp/pipkin-v1-baseline/{workspace,clippy,fmt,e2e,package,verify-install,installer}.log`. These logs are temporary and are not committed; the durable evidence summary is this document. Archive fresh logs/artifact checksums with CI and release candidates in Phase 2/6.

## First Phase 1 fix (2026-10-06)

Confirmed that Pi's current experimental session service has no rename operation. Real-mode Rename is now
disabled in the command palette; F2 reports “Renaming conversations is not supported by this Pi engine”
instead of opening a dialog that silently does nothing. Demo rename remains enabled. A regression test
checks both modes.

Post-fix validation: **506 workspace tests passed (36 ignored)**, Clippy and formatting passed. This is a
working-tree change on top of the identified baseline above, not a rebuilt/requalified release artifact.
The toast/disabled palette state has not yet been walked in the native installed UI.

## Gate tracker

Status meanings: **passed (automated)**, **owner-reported**, **partial**, **open**, **failed**, or **unverified**. Every candidate should update these with its app/engine revisions and evidence. Historical passes must not silently become qualification of a changed candidate.

| Gate | Phase | Current status | Ownership / next action |
| --- | --- | --- | --- |
| Release scope and final acceptance policy | 0 | Working scope recorded; final sign-off open | Owner confirms advertised support and any acceptance exceptions |
| Test/package baseline | 0 | Passed (automated), identified above | Engineering reruns after code/candidate changes |
| Daily-use beta | 0 | Owner-reported | Owner supplies dates/incidents if available; do not invent metrics |
| Real-mode rename | 1 | Addressed in working tree: palette disabled, F2 explains; automated regression passed | Native installed walkthrough remains; authoritative rename is not advertised |
| Changes scan failure | 1 | Open: unavailable result becomes empty changes | Engineering retain stale result and expose failure/retry |
| Interrupted tools/unresolved operations | 1 | Open | Engineering audit recovery messaging and safe reconciliation actions; no blind replay |
| Recent UI regression walkthrough | 1/4 | Partial: targeted automated/native evidence exists | Engineering + owner verify menus, effort, pane sizing, and mentions in installed build |
| Pinned clean engine build and manifest | 2 | Open: baseline identified, build still uses adjacent checkout | Engineering pin full revision, verify clean/generated inputs, and reject unidentified release sources |
| CI and retained evidence | 2 | Open: no repository CI workflow established | Engineering automate tests, packaged-engine qualification, and retained artifacts/logs |
| Runtime/dependency/license audit | 2 | Open | Engineering check bundled licenses, generated assets, minimum Node, and runtime dependencies |
| Clean installed desktop workflow | 3 | Unverified | Owner/second supported environment: actual package installation, launcher, provider, real reply |
| Real app upgrade/rollback and schema backup restore | 3 | Partial: automated migration/installer tests; manual installed workflow open | Owner/engineering use disposable profile and actual old/new packages |
| Minimize/restore | 4 | Unverified in gate record | Owner runs documented native procedure |
| Suspend/resume during work | 4 | Unverified in gate record | Owner runs documented native procedure; verify reconnect and no duplicate client dispatch |
| Physical monitor unplug/replug | 4 | Open: layout regression covered, physical transition not recorded | Owner tests actual transition and reachable splitters |
| Scale/text/layout matrix | 4 | Partial: historical observations at 125%/1.6; full matrix not recorded | Owner/engineering check both themes, enlarged text, narrow layouts, focus and popups |
| IME commit/cancel | 4 | Partial: historical pinyin commit observation | Owner verifies actual commit/cancel and candidate geometry, including path completion |
| Keyboard + Orca workflow | 4 | Partial; typed-character/caret speech previously unresolved | Engineering fix/verify; owner runs complete native workflow |
| Picker/drop/clipboard/links/launching | 4 | Partial; not fully walked on installed candidate | Owner performs real interactions; engineering fixes findings |
| Engine-unavailable native state | 4 | Automated behavior covered; native installed observation open | Owner/engineering check actionable error, cached history, and disabled dispatch |
| Cold reboot/start | 4 | Unverified | Owner records recovery and usable startup, not just window mapping |
| Onboarding/diagnostics/compatibility docs | 5 | Open | Engineering add safe Copy diagnostics and update setup/support/extension boundaries; observe new users where possible |
| Extended soak + real-window day | 6 | Open; 40-prompt baseline tripwire passed, historical 1500-prompt measurement exists | Engineering + owner run documented longer workloads and record app/engine resources |
| Real signing key and independently verifiable candidate | 6 | Open; signature tooling previously exercised with throwaway key | Owner manages real key; engineering builds/verifies candidate |
| RC stabilization and final release | 6/7 | Not started | Owner approves gates; engineering publishes identified signed artifacts and update/recovery policy |

## Immediate execution queue

1. Make unavailable Changes scans explicit and preserve the correct project's stale result.
2. Audit interrupted-tool and unresolved-stop messaging/recovery.
3. Establish the pinned clean release engine and CI so subsequent fixes are qualified against a repeatable build.
4. Schedule owner-run clean-install and native gates while onboarding/support work proceeds.

For step details and exit criteria, use the sequenced plan. `docs/native-gate-testing.md` remains the manual procedure reference; unrecorded owner use must not be assumed to have passed a particular gate.
