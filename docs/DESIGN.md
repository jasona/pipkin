# Pi Desktop design record

Derived from the shipped build (`crates/desktop-ui/src/theme.rs`, `shell/controls.rs`). Quiet, precise developer workspace: neutral layered surfaces, subtle separators, one restrained accent, text-and-icon status. Zed is the interaction-quality reference, not a visual template.

## Layout
- Nominal 1440×960: navigation 240 px · conversation (flexible, text column bounded to 760 px × text scale) · inspector 400 px. Dividers drag; widths persist (nav 180–420, inspector 280–720).
- Docking rules (`shell/workspace.rs`): navigation docks at ≥ 900 px wide; the inspector docks only when the conversation keeps ≥ 480 px. Otherwise they become temporary, mutually exclusive drawers over a scrim; Escape closes and focus returns to where it was.
- Verified sizes: 1440×960, 1024×768 (inspector temporary), 720×800 (both temporary).

## Tokens (semantic; dark / light share the same set)
Surfaces `bg_app < bg_pane < bg_surface`, `bg_elevated` only for menus and dialogs. States `bg_hover`, `bg_active`, `bg_selected`. Lines `border`, `border_strong`. Text `text`, `text_muted`, `text_faint`. One accent (`accent`, `accent_bg`, `text_selection`). Status `success`, `warning`, `danger`(+`_bg`). Code `code_bg`, `syn_*`. Diff `diff_add_*`, `diff_remove_*`, `diff_hunk_bg`. Dark accent `#8aa4ff`, light accent `#3454d1`.

## Type and spacing
- 4 px rhythm. Compact controls 30 px (24 px compact). UI text 14 px, small 12 px, conversation 15.5 px / 1.55, code 13 px / 1.55, all × text scale (Small 0.92, Normal 1.0, Large 1.2).
- Bundled fonts: IBM Plex Sans (UI/prose) and Lilex (code), both OFL; see `assets/PROVENANCE.md`. Icons: Lucide (ISC).
- Known gap: inline code uses the prose size, so Lilex looks large against Plex Sans inside sentences (GPUI text runs cannot change size).

## Components (`shell/controls.rs`)
`Btn` (kinds Ghost / Subtle / Primary / Danger, compact, icon-only, selected, disabled), `chip`, `kbd`, `elevated`, `menu_row`. States: hover, active, selected, disabled (40% opacity, not-allowed cursor), keyboard focus (`focus_visible` accent ring, tab stop), loading (static spinner icon), error (danger text + icon). Enter/Space activate focused buttons.

## Conventions
- Authorship: user prompt is the only card; Pi replies are plain flowing markdown; tool activity is compact expandable rows; notices are single lines.
- One primary action per surface (Send, or Steer while running). Stop is Danger.
- Motion: none continuous. Streaming and "Working" are static; reduced-motion also stops caret blinking.
- Text scale and theme live in Preferences (Ctrl+,) and the palette.
- Demo honesty: a "Demo · simulated agent" chip is always visible; the inspector reads "Workspace changes · Demo".
