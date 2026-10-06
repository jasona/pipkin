# macOS: experimental first pass

The `Experimental macOS build` GitHub Actions workflow builds the actual GPUI app on a macOS 15
runner. Download the artifact identified by its app revision and runner architecture, unzip its
experimental app archive, and retain `build-info.json` and `SHA256SUMS`. This is a native single-
architecture build, **not** a universal binary, signed/notarized distribution or supported 1.0 release.
Do not treat a checksum as publisher authentication.

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
