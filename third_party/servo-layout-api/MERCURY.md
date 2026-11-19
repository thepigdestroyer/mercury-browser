# Patched servo-layout-api 0.8.0

This directory vendors `servo-layout-api` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`3ba7c3c73cc25b20dbdf2f6b04a886a6dc595dc7d11c8b412200570d04f1555e`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/shared/layout`,
plus Mercury's changes in `third_party/patches/servo-layout-api.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-layout-api` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026).

### Caret in contenteditable (servo/servo#48253) — `lib.rs`

- **What:** backport of servo/servo#48253 "Render caret inside of contenteditable elements" (merge commit `cd332167d`).
  `GenericLayoutDataTrait::set_text_run_selection` takes a second argument, `new_paints_caret: bool` (`lib.rs`, about line
  94). Implemented in `servo-layout` (`dom.rs`) and called from `servo-script` (`dom/characterdata/characterdata.rs`).
- **Why:** see `third_party/servo-layout/MERCURY.md`.
- **Tested by:** `tests/pages/compat-editing.html`.
- **Drop when:** the Servo bump that contains PR #48253 (B1).

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-layout-api.patch` and this directory
once Mercury no longer needs to change the crate.
