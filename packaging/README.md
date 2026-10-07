# Pipkin installed help

This package carries the app, paired Pi engine and pinned private Node runtime. It does not install system Node, Git or graphics/desktop libraries.
Current packages are qualification builds; final native acceptance and release approval remain open. Do not assume
an unsigned artifact is authenticated or portable to another distribution merely because it can be unpacked.

- [Getting started and provider setup](docs/getting-started.md)
- [Data ownership, diagnostics privacy and recovery](docs/support.md)
- [Private security reporting](SECURITY.md)
- [Install/update/rollback and schema backups](docs/packaging.md)
- [Platform, engine compatibility and runtime requirements](docs/platforms.md)
- [Extension compatibility](docs/extensions.md)
- [Bundled notices and native/runtime inventory scope](docs/bundled-licenses.md)
- [Release evidence and remaining gates](docs/v1-release-gates.md)

`build-info.json` identifies this package's app/engine/build inputs. `runtime-inventory.json` records direct native
library/ABI references; `third-party/inventory.json` records conservative package notices and open review flags.
Neither inventory is legal clearance, a complete binary SBOM or a guarantee that all optional native components work.

Use **Ctrl K → Copy diagnostics** for limited metadata. Detailed `pipkin --diagnose` output can include sensitive
log/error text. Review it before sharing. Never upload credentials, a full database or private sessions by default.

The `docs/` directory also includes explicitly historical development notes. Their old measurements/commands do
not qualify this artifact. Source-build/developer commands require the full repository; screenshots, fixture captures,
source scripts and the full source checkout are not included in installed help. Consult the
[public source repository](https://github.com/last-refuge/pipkin) for those materials.

Keep the previous known-working package and private app/engine backups. Quit/restart, Stop and rollback do not undo
completed file or external effects. Follow the recovery guide before repeating uncertain work.
