# Packaging, install, upgrade and diagnostics

## What gets installed

```
/usr/bin/pipkin                         the application
/usr/lib/pipkin/engine/                 a self-contained Pi engine (sources, production node_modules, engine.json)
/usr/share/applications/pipkin.desktop  launcher entry (StartupWMClass=pipkin)
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

The engine is staged from the Pi fork checkout by `scripts/build-engine.sh`: a copy without `.git` and the
evaluation suite, dev dependencies pruned, plus the manifest. It is source plus dependencies run by Node's
TypeScript support, as Pi's own `pi-test.sh` does; Pi does not yet publish a compiled experimental server, so a
compiled engine is a Pi-side follow-up. The installed size is large (about 850 MiB) because of that.

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
