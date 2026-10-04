# Pi Desktop

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

Native desktop software. The current prototype acceptance target is Omarchy/Hyprland on Wayland, confirmed by the user. Windows and macOS are future validation targets; this pass does not establish support for them.

## Stack

Rust and GPUI, evaluated through a running prototype. Keep application state and execution contracts independent of GPUI. The downloaded `zed/` source is an implementation reference.

## Product Purpose

A desktop interface for Pi's coding-agent workflow: open a project, describe work, follow progress, inspect changes, continue or stop, and return without losing context.

## Users

The existing product plan assumes individual developers working locally. The immediate evaluator is the project owner on Omarchy/Wayland; broader audience validation remains open.

## Capabilities and Constraints

- This pass must produce a high fidelity running prototype whose interface and application foundation can be retained for real integration.
- This planning task does not implement the application.
- The longer-term product direction is recorded in `rust-desktop-client-plan.md`.
- The prototype should make framework limitations visible through real interaction and measurable checks.
- Real Pi integration and its exact service contract remain future work; the engine source paths cited by the original plan are absent from this checkout.

## Evidence on Hand

- `rust-desktop-client-plan.md`: product workflow, proposed architecture, visual direction, and release requirements.
- `zed/`: local source reference at revision `a84689073d296dfd39987bc7dd478e43ef76d83a` when inspected.
- No existing Pi desktop implementation or approved application screenshots were found in this workspace.

## Product Principles

- Prioritize reading, writing, and understanding agent progress.
- Preserve drafts and report uncertain outcomes honestly.
- Keep simulated execution behind the same application boundary intended for a future real adapter.
- Evaluate usability and text interaction alongside appearance.

## Accessibility & Inclusion

The original plan requires keyboard operation, visible focus, scalable text, contrast, reduced motion, and screen-reader workflows. The prototype must gather evidence on these requirements; source-level framework support alone is insufficient.
