# Pipkin (GPUI prototype)

Native Rust/GPUI desktop client for Pi. Started as a single-pass prototype ([`docs/gpui-prototype-plan.md`](docs/gpui-prototype-plan.md)); now being wired to a real Pi engine per [`docs/rust-desktop-client-plan.md`](docs/rust-desktop-client-plan.md). **Start there: its "Implementation status" section says where work stands and how to resume.**

```sh
# system: Wayland compositor, Vulkan driver, fontconfig, xkbcommon; Rust 1.99.0 (rust-toolchain.toml)
# real mode (default): a client for a running Pi experimental server, or one it launches and owns (--pi-repo)
cargo run -p pipkin-app --release -- [--pi-dir DIR] [--pi-server-id UUID] [--pi-repo DIR --pi-agent-dir DIR [--pi-extension DIR]...] [--project DIR] [--data-dir DIR] [--editor CMD] [--terminal CMD]
# simulated agent, for regression and demos only
cargo run -p pipkin-app --release -- --demo normal [--data-dir DIR] [--speed 1.0]
cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --all --check
```

Real mode needs a Pi engine; `scripts/try-m2.sh <pi checkout>` launches the app against a managed engine and a scripted offline provider (a prompt containing "slow" holds the run so it can be steered, queued and stopped). It needs a Pi server; `scripts/pi-test-server.sh` starts an isolated throwaway one (setup notes are in the plan's "Implementation status"). The opt-in real-server tests are `cargo test -p pipkin-app real_pi -- --ignored`.

Extensions that ask questions (the example is `packages/coding-agent/examples/plugins/pi-example-questions` in the Pi fork) are described in [`docs/extensions.md`](docs/extensions.md). `--editor` and `--terminal` set the commands behind "Open in editor" and "Open a terminal" (otherwise `$VISUAL`/`$EDITOR`, `$TERMINAL`, then the desktop's). Without an engine, opened conversations still open from their saved copies.

Scenarios: normal, followup, failure, unknown, stressed, large, persist-fail ([`docs/scenarios.md`](docs/scenarios.md)). Palette: Ctrl+K.

Docs (all in [`docs/`](docs/)): `review-script.md`, `scorecard.md`, `architecture.md`, `decisions.md`, `baseline.md`, `continuation-map.md`, `DESIGN.md`, `PRODUCT.md`, `rust-desktop-client-plan.md`. Agent instructions: [`AGENTS.md`](AGENTS.md).

The `zed/` directory is an optional read-only checkout of Zed (rev `a846890`) used only for reading source; it is git-ignored and the build does not need it (GPUI is pinned by git revision).
