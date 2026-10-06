# Packaging, install, upgrade and diagnostics

## What gets installed

```
/usr/bin/pipkin                         the application
/usr/lib/pipkin/engine/                 a self-contained Pi engine (sources, production node_modules, engine.json)
/usr/share/applications/pipkin.desktop  launcher entry (StartupWMClass=pipkin)
/usr/share/licenses/pipkin/             MIT licence for Pipkin, plus the font and icon licences
/usr/share/icons/hicolor/{128,256,512}x…/apps/pipkin.png
```

The engine runs on the system `nodejs` (>= 22.19, a package dependency). `engine.json` records the engine
version, the protocol it speaks (8) and the oldest Pipkin it supports. Pipkin refuses an engine with another
protocol, an incomplete one, or one that needs a newer Pipkin, and says why (`pipkin --diagnose`).

With no flags, Pipkin looks for the engine at `PIPKIN_ENGINE_DIR`, then `../lib/pipkin/engine` relative to its own
binary (so a relocated prefix works), then `/usr/lib/pipkin/engine`. When it finds one it launches and owns it:
profile under `~/.pi/server`, credentials and sessions under Pi's own `~/.pi/agent`. `--pi-repo` (a checkout) and
`--pi-dir` / `--pi-server-id` (an engine already running) still win, for development.

## Building

```
scripts/package.sh [PI_CHECKOUT]      # dist/pipkin-<ver>-<arch>.tar.zst and dist/stage/
cd packaging && makepkg -d -f         # an Arch package from the same steps (PI_CHECKOUT=../../pi-fork/pi)
scripts/verify-install.sh [PKG]       # unpack into a scratch prefix and check it with a bare environment
scripts/verify-install.sh --full      # ...and run the real-engine workflow tests against the unpacked engine
```

Release engine sources must match the full revision in `packaging/pi-engine-revision` and have a clean Git
working tree. `scripts/check-engine-source.sh PI_CHECKOUT` verifies both before staging. Prepare a clean checkout
of that revision with `npm ci`, then `npm run hydrate:model-data` at the Pi root if generated provider data is missing.
`PIPKIN_ENGINE_DEV=1 scripts/package.sh PI_CHECKOUT` is an explicit development override, not release qualification.

`scripts/build-engine.sh` stages the identified tracked sources through Git archive, not arbitrary ignored build
output. It separately validates/copies required generated provider JSON and installed workspace dependencies,
then prunes dev dependencies offline. The manifest records the full source revision, generated-data manifest hash,
protocol, minimum client and whether the development override was used. It does not yet identify every build input
or certify third-party dependency provenance; those audits and CI qualification remain release gates.

The engine is source plus dependencies run by Node's
TypeScript support, as Pi's own `pi-test.sh` does; Pi does not yet publish a compiled experimental server, so a
compiled engine is a Pi-side follow-up. The original installed-size measurement was about 850 MiB. The
2026-10-06 v1 baseline stages **453 MiB total**, with **200 MiB of engine** and an **89,114,048-byte tarball**
(about 85 MiB). Sizes vary with engine dependencies and binary debug information; see
[v1-release-gates.md](v1-release-gates.md) for the exact identified artifact.

`makepkg` needs `!lto` (set in the PKGBUILD): its LTO flags break linking the bundled SQLite.

## Upgrade and rollback

- **Application and engine upgrade together**: one package replaces `/usr/bin/pipkin` and `/usr/lib/pipkin/engine`, so
  they never disagree. A running engine from the old package is left to finish; the new Pipkin starts its own.
- **Data upgrades**: Pipkin's database migrates in place on first start. Before migrating it writes a backup of the
  old schema next to the database; a test upgrades from every earlier schema and checks the draft survives and a
  backup exists.
- **Rollback**: reinstall the previous package (`pacman -U` from the cache). A database written by a newer Pipkin
  is refused untouched with the message "schema N is newer than this build supports"; restore the backup made at
  that upgrade (the `.bak` for the older schema) to go back. Drafts typed after the upgrade are not in that backup.
- **Damage**: a corrupt database is set aside and the newest intact backup restored, with a notice.

## Diagnostics

```
pipkin --version
pipkin --diagnose            # versions, paths, database health, engine manifest and checks, Node, display, log tail
pipkin --diagnose --probe    # also starts a throwaway engine (offline, empty profile) and checks it answers
```

The report replaces the home directory with `~`, never prints credentials (it reads none), exits non-zero when
something blocks Pipkin, and is safe to paste into an issue. The engine's own output is in
`<data dir>/engine.log` (default `~/.local/share/pipkin/`).

## Soak

`PIPKIN_PI_REPO=<engine dir> PIPKIN_SOAK_ROUNDS=1500 cargo test -p pipkin-app --release many_prompts -- --ignored --nocapture`
sends prompts through one conversation of a real engine and fails if memory or open files keep growing. A
1500-prompt run on the bundled engine: app 22 -> 52 MiB, engine 633 -> 700 MiB, open files 15 -> 19.

## Generic Linux install, upgrade and rollback

`scripts/package.sh` also leaves `install.sh` at the root of the release tarball. It installs into `~/.local`
(or `--prefix DIR`) under `lib/pipkin/versions/<version>/` with a `current` link, so:

```
./install.sh                # install or upgrade; keeps the newest 2 versions (--keep N)
./install.sh --list         # versions, with the current one marked
./install.sh --rollback     # switch back to the previous version in one step
./install.sh --uninstall    # remove links and versions; your drafts and history are not touched
```

Each version carries its own engine (the binary finds it relative to itself), so an upgrade never leaves an
app and an engine that disagree, and the old version is a working fallback. `scripts/test-install.sh` checks
install, upgrade, list, rollback and uninstall in a scratch prefix, including that data survives.

## Releases: checksums and signatures

```
scripts/release.sh                       # dist/release/: the tarball and SHA256SUMS
PIPKIN_SIGN_KEY=<gpg key id> scripts/release.sh   # also SHA256SUMS.asc (detached signature)
scripts/verify-release.sh DIR [--require-signature]   # what a downloader runs
```

Checked here with a throwaway gpg key (signature verifies; a modified tarball fails the checksum). No real
release key exists, nothing is published, and there is no in-app update check: an update is "download, verify,
`./install.sh`", with `--rollback` as the bootable prior version.

## Display servers

The binary is built with both `wayland` and `x11`; it uses Wayland when `WAYLAND_DISPLAY` is set, else X11.
The Arch package depends on `libxkbcommon-x11` and `libxcb` for that. See `platforms.md` for what was checked.
