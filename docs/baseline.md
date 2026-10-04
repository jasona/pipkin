# Reference machine baseline

Recorded 2026-10-03 (step 1, native foundation).

| Item | Value |
| --- | --- |
| OS | Omarchy, Linux 7.2.5-3-omarchy |
| Compositor | Hyprland 0.56.2 (v0.56.2, efb5099) |
| Display | HDMI-A-1, 2560x1080 @ 60 Hz, scale 1 (125%/150% unverified) |
| GPUs | Intel Raptor Lake-S UHD (RPL-S, 0x8086:0xa788); NVIDIA RTX 5070 Laptop (0x10de:0x2d18) |
| GPU selected by GPUI | Intel RPL-S, Vulkan (follows the compositor GPU hint) |
| Rust | 1.99.0 (installed stable); pinned in `rust-toolchain.toml` |
| System libs | wayland-client 1.26.0, xkbcommon 1.13.2, vulkan 1.4.357, fontconfig 2.18.3 |
| Window | native Wayland (`xwayland: False`), app_id `pipkin`, 1440x960 |

Launch the demo: `cargo run -p pipkin-app --release -- --demo normal`. Without `--demo` the app starts in real mode, which has no engine adapter yet and shows an offline state (set `RUST_LOG=info` for adapter logs).

Capture caveat: Omarchy's default window-opacity rule makes the app translucent, so other windows ghost through captures. Not an app setting.
