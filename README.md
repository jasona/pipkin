# Pi Desktop (GPUI prototype)

Single-pass prototype per `gpui-prototype-plan.md`: native Rust/GPUI app with a simulated Pi backend.

```sh
# system: Wayland compositor, Vulkan driver, fontconfig, xkbcommon; Rust 1.99.0 (rust-toolchain.toml)
cargo run -p desktop-app --release -- --demo normal [--data-dir DIR] [--speed 1.0]
cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --all --check
```

Scenarios: normal, followup, failure, unknown, stressed, large, persist-fail (`docs/scenarios.md`). Palette: Ctrl+K.
Docs: `docs/review-script.md`, `docs/scorecard.md`, `docs/architecture.md`, `docs/decisions.md`, `docs/baseline.md`, `docs/continuation-map.md`, `DESIGN.md`.
