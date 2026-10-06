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

## Changes scan hardening (2026-10-06)

Implemented on top of **`4a6e633ad82a00d1225ff3dc4dede33cc830fed9`**, as an uncommitted working-tree delta.
Engine remains **`d4c871ef75bea57f43518e5e554c5a4765173a0b`**, tracked working tree clean at recheck.
Database schema remains 7. Source delta identifier (`git diff --binary -- crates/ | sha256sum`):
`93e1662b58fa90d35813f530a9fa78a4771467113ce05ba68ee016b7b72b56ce`.
This identifies the tested delta, not a clean release revision.

Behavior:

- Fresh untouched conversations still begin with the calm empty inspector; automatic scanning waits for work.
- Loading, a successful empty scan, unavailable scans, stale retained diffs, and non-Git folders are distinct.
- Failed scans expose the reason and retain only that conversation's last successful diff, explicitly marked out of date.
- Inspector refresh, failure-state Retry scan, and the palette's Refresh workspace changes share core availability.
  Explicit refresh can scan an untouched session, is read-only, and cannot stack duplicate client requests.
- Backend scanning remains off the UI thread, single-flight with one coalesced follow-up; generation/current-session
  guards prevent an old attachment's result from appearing as another project's state.
- Reattachment does not erase the previous diff before a successful scan. Refresh preserves selected paths rather
  than an obsolete row index. A scan revision invalidates cached diff text even when line counts stay the same.
- Missing Git/failed process launch is no longer classified as a non-Git folder.

### Automated evidence

- **511 workspace tests passed, 37 ignored**; Clippy/fmt clean.
- **34 e2e suite tests passed** against the development engine, serial; the new regression confirms that a
  missing project does not erase its diff, retry updates it, a non-Git folder is explicit, and scans send no prompt.
- Rebuilt package; `verify-install.sh --full` passed with **34 e2e suite tests against the unpacked engine**.
  `test-install.sh` passed all 12 scratch-prefix checks again.
- Targeted socket/mock-engine tests also cover malformed Git config, missing directory, non-Git folders,
  untouched-session behavior, repair/retry, routing, and no submit/steer/stop side effects.
- Core/UI-copy tests cover generation and project isolation, stale snapshot preservation, offline/interrupted
  refresh, duplicate retry suppression, equal-count diff cache invalidation, selected-path preservation, and
  distinct empty/progress/failure/non-Git wording.

Suite counts include the opt-in profile-seeding helper, which returns without seeding when `PIPKIN_SEED_ROOT`
is unset; they are not claims of 34 distinct engine scenarios. Seeding was separately executed for the native check.

Rebuilt `pipkin-0.0.1-x86_64.tar.zst`: **89,133,924 bytes**; SHA-256:

```text
0bcf98585fb27d9d805cc0fea100a089519c97a86daa43902dcfb41f711a35d0  pipkin-0.0.1-x86_64.tar.zst
```

This replaced the local `dist` baseline tarball. Both recorded checksums describe their respective builds;
this artifact is still unsigned, version 0.0.1, and not a release candidate or a clean-system desktop pass.

### Observed native evidence (not a screen-reader speech or visual-matrix pass)

Launched the current release binary with a disposable seeded profile/project on the existing Hyprland/Wayland
session. All keyboard input used `scripts/guard.sh` pinned to that launched PID, with an isolated guard-state file.
Read the launched process's AT-SPI tree and recorded:

1. Malformed project Git config: “Changes unavailable,” the actual Git error, “No current diff is available,”
   and an accessible Retry workspace changes scan button.
2. Restored config and guarded palette refresh: `notes.txt, 1 added, 0 removed`, with the failure banner absent.
3. Failed another scan: explicit “Showing the last successful scan … out of date” plus the same selected diff/file.
4. Restored config, requested refresh, and quit; the launched application and owned engine exited.

No prompt was sent in this native walkthrough. The user's profile/configuration was not modified. AT-SPI initially
showed an unnamed status container; the same bounded check found and corrected its accessible label before the
final observation above. Actual Orca speech, visual contrast/layout, mouse activation of Retry, and the broader
native gates remain unverified here.

Temporary logs: `/tmp/pipkin-changes-{workspace,clippy,e2e,package,verify-install,installer,seed}.log`.
Native tree captures remain under the disposable `/tmp/pk-changes-JzYiFq/` profile. They are local/temporary;
this summary is the durable gate evidence. Future CI/candidate runs must retain fresh evidence.

## Stop retires obsolete instructions (2026-10-06)

Owner-reported regression: submit “commit and push,” Stop, then enter a new prompt or `/goal count to 100`;
the agent finished the stopped request before handling the replacement.

Confirmed engine defect: aborted assistant messages were excluded from model context, but their original user
instructions remained. New durable settlement appends a context-omit edit atomically with an aborted placed input.
This retires the instruction from future model requests without deleting visible history or completed tool effects.
Queued inputs are still withdrawn by the engine's abort operation. This applies to all prompts, not only goals.

Pi fix committed/pushed on `client-history-and-ui-requests`:
**`d2a311097cbcf669e699479587332ae3988a49d0`**. Full `npm run check` passed; **223 targeted durable tests passed**
across cancellation/context, submissions, generation/recovery, inbox, conversations, compaction, tables and forks.
The new prompt/goal regressions failed before the fix and passed after it. Tests also verify durable reopen,
atomic settlement/retirement, idempotent repeated Stop, preservation of original entries, and withdrawal of queued work.
Tests used a temporary Vitest source-alias configuration because this checkout's AI package has no built `dist`;
no dependency installation or unrelated package build was performed.

Pipkin changes on top of **`a3ee2bd5d9feb86788a1118791b14b4895455bc4`**:

- Stop pauses the goal present at the instant it is requested, preventing continuation if completion races the stop.
- A replacement goal entered while stopping waits for confirmation, then starts without being paused by the old ack.
- A fresh goal also waits for the engine-owned queue to clear.
- The adapter synchronizes the latest replicated transcript before terminal settlement, so goal verdict parsing
  does not race the final reply's queued notification.

Tested source delta (`git diff --binary -- crates/ | sha256sum`):
`03f46b06a716420d50df117f6b697322f8e8e90db37f23d0b9509b81847c52ed`.

Validation: **523 workspace tests passed (40 ignored)**, Clippy/fmt clean; **37 real-engine e2e suite tests passed**
against the updated Pi checkout and **37 passed against the rebuilt bundled engine**. Suite counts include the
opt-in seeding helper, a no-op when its environment flag is unset. Both replacement-prompt and replacement-goal
regressions assert that the next scripted-provider request contains only the new task, not the stopped request or
withdrawn follow-up; the stopped prompt remains visible in the transcript. Scratch-prefix installer checks passed.
This is automated engine/controller qualification, not a new native keyboard/mouse or real-provider walkthrough.

Rebuilt `dist/pipkin-0.0.1-x86_64.tar.zst`, still unsigned and not a release candidate:

```text
5ac8cd98fccbd944e603db63a835a0543db11e60575dd686f43c4348686aec1f  pipkin-0.0.1-x86_64.tar.zst
```

**Deployment requires the updated engine as well as the app.** Rebuild/reinstall the bundled package and restart;
an app-only update against the old engine retains the context defect. Existing tool effects, commits or pushes
cannot be undone by Stop. No installed app, owner profile, or desktop configuration was changed during qualification.
Local temporary logs: `/tmp/pipkin-stop-{workspace,clippy,e2e,targeted,package,verify-install,installer}.log`.

## Phase 2: pinned source staging (2026-10-06)

Implemented on app base `55f90b0c0ad713f2fb4125058929f09705f529af` with Pi pin
`d2a311097cbcf669e699479587332ae3988a49d0` in `packaging/pi-engine-revision`:

- Default engine staging rejects unidentified, dirty/untracked, and unpinned Git sources before replacing output.
  Development overrides are explicit (`PIPKIN_ENGINE_DEV=1`) and identified in the manifest; release collection
  refuses this override.
- Staging archives tracked sources instead of arbitrary ignored checkout output, copies installed workspace
  dependencies, and validates/copies the required generated provider JSON. The manifest records the full source
  revision and generated-data manifest SHA-256, not only a short commit.
- Release collection selects the current version/architecture, not every old tarball in `dist`.
- Disposable source/staging tests cover pin matching, dirty/untracked/mismatched/invalid pin rejection, the explicit
  override, missing generated input without destroying prior output, preserved dependencies and exclusion of stale
  ignored output. Disposable release tests verify development rejection and exclusion of obsolete artifacts.

An initial Git-archive-only build **failed packaged qualification**: provider JSON is Git-ignored but mandatory.
After explicit asset validation/staging, **37 packaged-engine suite tests passed**, installer scratch-prefix checks
passed, and **531 workspace tests passed (40 ignored)** with Clippy/fmt clean. The suite includes the no-op opt-in
seeding helper. No clean desktop install, real-provider onboarding, accessibility or release acceptance is claimed.

The local rebuilt unsigned 0.0.1 artifact SHA-256 is:
`91484e0c573b0e8a4d83cef4efbc5057f75628ea00628f48a5bc5f5fec08e890`.
Its engine generated-data manifest SHA-256 is:
`b92d631bb8bb2cac6f824a03a814baad9452164c518426b8bc4bbc5c2142bb05`.

Phase 2 remains **partial**: CI, complete app/build-input identification, clean-checkout provisioning, and dependency
license/provenance auditing still need work. This is identified staging, not a claim of byte-for-byte reproducible
builds. Temporary logs: `/tmp/pipkin-pin-{package,verify,installer,workspace,clippy}.log`; the final verification log
records the corrected pass, while the failure cause/count above preserves the initial failure evidence.

## Gate tracker

Status meanings: **passed (automated)**, **owner-reported**, **partial**, **open**, **failed**, or **unverified**. Every candidate should update these with its app/engine revisions and evidence. Historical passes must not silently become qualification of a changed candidate.

| Gate | Phase | Current status | Ownership / next action |
| --- | --- | --- | --- |
| Release scope and final acceptance policy | 0 | Working scope recorded; final sign-off open | Owner confirms advertised support and any acceptance exceptions |
| Test/package baseline | 0 | Passed (automated), identified above | Engineering reruns after code/candidate changes |
| Daily-use beta | 0 | Owner-reported | Owner supplies dates/incidents if available; do not invent metrics |
| Real-mode rename | 1 | Addressed in working tree: palette disabled, F2 explains; automated regression passed | Native installed walkthrough remains; authoritative rename is not advertised |
| Changes scan failure | 1 | Addressed in working tree: automated/core/real-engine/package checks passed; native AT-SPI failure/stale/recovery observed | Broader native visual/Orca/mouse checks remain open; repeat on the final candidate |
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

1. Audit interrupted-tool and unresolved-stop messaging/recovery.
2. Establish the pinned clean release engine and CI so subsequent fixes are qualified against a repeatable build.
3. Schedule owner-run clean-install and native gates while onboarding/support work proceeds.

For step details and exit criteria, use the sequenced plan. `docs/native-gate-testing.md` remains the manual procedure reference; unrecorded owner use must not be assumed to have passed a particular gate.
