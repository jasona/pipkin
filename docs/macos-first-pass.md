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
“satisfies its Designated Requirement”). That run fixed the signature defect; it did not include Pi/Node. For the newer self-contained build below,
extract a fresh app and replace the old bundle rather than merging files into it.
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
- Starting with clean `c91bbfe17e27d5f42ebceda9c0784d8bd5c93e6e`: the actual pinned Pi engine
  (`d2a311097cbcf669e699479587332ae3988a49d0`), immutable model snapshot, production npm dependencies
  with retained notices, and architecture-matched Node **22.23.3** with its full dependency LICENSE.
  Node/npm are not installed globally; the engine uses the app's private runtime, not Terminal's PATH.

Self-contained qualification: [37507592573](https://github.com/last-refuge/pipkin/actions/runs/37507592573).
Artifact: `pipkin-macos-ARM64-c91bbfe17e27d5f42ebceda9c0784d8bd5c93e6e`.
Archive SHA-256: `31c95536ce8e82c2c7d86faa15b4185029247dfce4d96294a8331a363cc46590`.
The native runner tested the staged bundled pair, then extracted the signed archive and ran its actual CLI
with a bare system PATH/private profile: Node 22.23.3, owned engine handshake **OK (1.9s)**. Strict signatures
passed before archive, after extraction and after engine startup. This is automated CLI/runtime evidence,
not observed Finder UI startup, publisher trust or provider login.

## Subscription onboarding build for owner evaluation

The OAuth-only onboarding implementation was built on macOS 15 (Apple Silicon) in [run 37559405162](https://github.com/last-refuge/pipkin/actions/runs/37559405162), clean commit `5fd4944f7a9a2be772dfe0d912677eed4c424bc7`. Download artifact `pipkin-macos-ARM64-5fd4944f7a9a2be772dfe0d912677eed4c424bc7`. Its experimental archive `pipkin-0.0.1-aarch64-apple-darwin-experimental.zip` has SHA-256 `5cc1367953d31471ebaf52f314dee4b798016bdd903a1583a7fb13bde66231a0`. This app includes the reviewed OAuth bridge in the bundled pinned Pi engine; CI passed its real local Unix-transport capability test, without using account credentials. The bundle is ad-hoc signed, **not** notarized; neither Finder launch nor live provider sign-in has been verified on a user's Mac. Use an isolated profile for first-run evaluation. Do not paste sign-in codes into logs or support reports.

## Experimental drag-to-Applications image

The first DMG run [37618114190](https://github.com/last-refuge/pipkin/actions/runs/37618114190)
**failed** after the image was created, mounted and copied: the copied app's signature and file
hashes passed, but its offline engine probe timed out after 45 seconds. The ZIP probe passed.
The DMG probe used a longer temporary profile prefix; the pinned Pi engine's longest internal
server socket path was 108 bytes in that run versus 104 for the successful ZIP probe, near or
above Darwin's Unix-socket limit. The log retained only the engine tail, not the bind syscall
error, so this is a strongly supported diagnosis, not direct bind-error evidence. Both probes
now use private short profiles and check Pi's *internal* socket-path budget before launching.
A new macOS CI pass is required; the first failed run produced no downloadable DMG artifact.

The packaging workflow now also builds `pipkin-<version>-<target>-experimental.dmg` from the **same
signed app** as the ZIP. The disk image contains `Pipkin.app` and a shortcut to `/Applications`.
The macOS runner mounts it read-only, copies the app to a disposable destination, checks its
signature and app/Node/icon hashes, and runs an offline engine handshake from the copied app.
`SHA256SUMS` lists both formats. This describes the new checks, **not a passing remote run or a
Finder installation**; examine the run for the exact revision before relying on its artifact.

For an owner evaluation, download both the DMG and `SHA256SUMS` from one identified Actions
artifact, run `shasum -a 256 -c SHA256SUMS` in their directory (the ZIP must also be present),
open the DMG in Finder, and drag **Pipkin.app** to **Applications**. Replace an earlier Pipkin.app
as a whole; do not merge its contents. Open the installed copy from Applications, not the mounted
image, and retain the app revision and macOS version with any observations. A checksum verifies
file identity against that artifact's report, **not the publisher**. This DMG is still ad-hoc
signed and unnotarized: it may be blocked by Gatekeeper. Do not disable Gatekeeper globally.
Developer ID signing, notarization, a quarantined-download walkthrough, Intel coverage and
interactive provider sign-in remain separate gates.

The initial CI checks the pure core, Unix transport (including kernel peer uid), macOS owned-process
identification and native compilation. It also checks owned engine startup/stop and one real
scripted-provider file-edit/history round trip. No paid/provider-authenticated request is required.
See the actual run logs: workflow existence alone is not a passing result.

## Launch for evaluation

To try the interface without an engine, use a disposable app data directory:

```sh
./Pipkin.app/Contents/MacOS/pipkin --demo normal --data-dir /private/tmp/pipkin-mac-demo
```

The demo is simulated, not an agent. The self-contained artifact requires **no Pi checkout, npm install,
system Node or manual server startup**. Real-mode engine/runtime evaluation can use a disposable app and
agent profile. The new first-run UI offers Claude Pro/Max and ChatGPT Plus/Pro OAuth only, and keeps Pi responsible for credential storage. Do not use personal credentials in unattended tests; interactive authorization and a first real reply still need owner observation.

For a disposable runtime evaluation (no credentials):

```sh
PI_CODING_AGENT_DIR=/private/tmp/pipkin-mac-eval-agent \
  ./Pipkin.app/Contents/MacOS/pipkin --data-dir /private/tmp/pipkin-mac-evaluation
```

The `--pi-repo`/`PIPKIN_ENGINE_DIR` override remains available for advanced development with a separately
prepared pinned checkout and compatible system Node. It is not required by the self-contained artifact.
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
