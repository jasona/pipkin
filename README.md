# Pi Desktop (GPUI prototype)

Single-pass prototype per [`docs/gpui-prototype-plan.md`](docs/gpui-prototype-plan.md): native Rust/GPUI app with a simulated Pi backend.

```sh
# system: Wayland compositor, Vulkan driver, fontconfig, xkbcommon; Rust 1.99.0 (rust-toolchain.toml)
cargo run -p desktop-app --release -- --demo normal [--data-dir DIR] [--speed 1.0]
cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --all --check
```

Scenarios: normal, followup, failure, unknown, stressed, large, persist-fail ([`docs/scenarios.md`](docs/scenarios.md)). Palette: Ctrl+K.

Docs (all in [`docs/`](docs/)): `review-script.md`, `scorecard.md`, `architecture.md`, `decisions.md`, `baseline.md`, `continuation-map.md`, `DESIGN.md`, `PRODUCT.md`, `rust-desktop-client-plan.md`. Agent instructions: [`AGENTS.md`](AGENTS.md).

The `zed/` directory is an optional read-only checkout of Zed (rev `a846890`) used only for reading source; it is git-ignored and the build does not need it (GPUI is pinned by git revision).
