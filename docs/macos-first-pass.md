# macOS: experimental first pass

The `Experimental macOS build` GitHub Actions workflow builds the actual GPUI app on a macOS 15
runner. First observed green run: [37482847877](https://github.com/last-refuge/pipkin/actions/runs/37482847877),
clean app `f52e8c254488c4524c4cb986530eee7048b4f138`, **Apple Silicon/aarch64**. Intel/universal builds
have not been produced or qualified. Download the artifact identified by its app revision and runner architecture, unzip its
experimental app archive, and retain `build-info.json` and `SHA256SUMS`. This is a native single-
architecture build, **not** a universal binary, Developer ID-signed/notarized distribution or supported 1.0 release.
Do not treat a checksum as publisher authentication.

## Downloaded-bundle signature qualification

The first f52e8c2/ba836a7 artifacts compiled and passed engine tests but did **not** sign/verify the completed
app bundle. The owner reported “damaged and can't be opened”; `codesign --verify` reported “code has no
resources but signature indicates they must be present.” Do not use those artifacts as launch-qualified builds.

Packaging now ad-hoc signs the completed `.app` and requires strict signature verification both before
archiving and after extraction. The embedded build report records the pre-bundle-signing binary hash; the
external `build-info.json` records the final signed executable hash, avoiding a self-referential resource seal.
Corrected run [37488722228](https://github.com/last-refuge/pipkin/actions/runs/37488722228) on clean
`91e1586000a1ed999d28b04fe9dbf4dcae316c83` passed both actual signature checks (“valid on disk” and
“satisfies its Designated Requirement”). Download its `pipkin-macos-ARM64-91e1586000a1ed999d28b04fe9dbf4dcae316c83`
artifact and extract a fresh app; replace the old bundle rather than merging files into it.
Ad-hoc signatures verify integrity, **not publisher identity or Gatekeeper acceptance**; Developer ID signing
and notarization remain absent.

Read-only checks on your extracted copy (use straight quotes, not smart quotes):

```sh
codesign --verify --deep --strict --verbose=4 "/absolute/path/Pipkin.app"
spctl --assess --type execute --verbose=4 "/absolute/path/Pipkin.app"
```

Do not disable Gatekeeper globally. A correct ad-hoc bundle may still be refused as unnotarized; that is a
separate trust/distribution gate, not something compilation or signature-integrity checks close.

## What is included

- `Pipkin.app`, with bundled UI assets, target-specific Rust dependency notices and exact source for
  identified MPL dependencies. Notice retention is not legal clearance.
- Source/binary/target identity and direct `otool -L` dependencies; this is not a complete native SBOM.
- The Pi source pin required for protocol compatibility. **The engine and Node are not bundled.**

The initial CI checks the pure core, Unix transport (including kernel peer uid), macOS owned-process
identification and native compilation. It also checks owned engine startup/stop and one real
scripted-provider file-edit/history round trip. No paid/provider-authenticated request is required.
See the actual run logs: workflow existence alone is not a passing result.

## Launch for evaluation

To try the interface without an engine, use a disposable app data directory:

```sh
./Pipkin.app/Contents/MacOS/pipkin --demo normal --data-dir /private/tmp/pipkin-mac-demo
```

The demo is simulated, not an agent. For real mode, use a **separate** checkout of the pinned Pi
engine (`d2a311097cbcf669e699479587332ae3988a49d0`), Node >=22.19 and Git. Install its dependencies
with `npm ci`, then restore the immutable model snapshot using the matching Pipkin source checkout's
`scripts/stage-model-data.sh ENGINE_ROOT`. That source helper currently requires GNU `sha256sum`,
GNU tar and zstd; on macOS, Homebrew's `coreutils`, `gnu-tar` and `zstd` supply them (put the GNU tar
`libexec/gnubin` directory on PATH). These are evaluator prerequisites, not bundled components.

Launch from a terminal whose PATH contains Node; Finder does not necessarily inherit that PATH:

```sh
./Pipkin.app/Contents/MacOS/pipkin --pi-repo /absolute/path/to/pinned-pi \
  --data-dir /private/tmp/pipkin-mac-evaluation
```

Choose a disposable project and deliberately configure provider credentials if making real requests.
See [getting started](https://github.com/last-refuge/pipkin/blob/main/docs/getting-started.md),
[support/privacy](https://github.com/last-refuge/pipkin/blob/main/docs/support.md) and
[security reporting](https://github.com/last-refuge/pipkin/blob/main/SECURITY.md)
in the source repository; the app's resource folder retains SECURITY.md. Gatekeeper may refuse the
unnotarized artifact: this first pass does not claim normal end-user installation or trust acceptance.

## Port boundaries and next checks

macOS uses `getpeereid` rather than Linux `SO_PEERCRED`. Detached engine discovery uses kernel uid
and exact NUL-delimited environment entries, never `ps` text or name matching. Uninspectable processes
are not signal targets. The Linux stale-lock-removal optimization is deliberately disabled on macOS;
Pi retains responsibility for its stale-lock arbitration there.

Native Finder startup, provider authentication, Gatekeeper/notarization, clipboard-image capture,
Cmd-key conventions, IME/accessibility, sleep/wake and interactive process cleanup still need actual
Mac observation. External editor/terminal defaults remain Linux-oriented; configure explicit argument
arrays where needed. The storage default is still the existing XDG-style location, not yet macOS
Application Support. These limits are not silently waived by a successful build.

Windows work and the final 1.0 version/tag/publication are deferred until the macOS build is reviewed.
