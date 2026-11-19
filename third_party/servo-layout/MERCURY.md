# Patched servo-layout 0.8.0

This directory vendors `servo-layout` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`02047fd51cca1b92cfd38bc4663c9f11f6ebf99cd2d5550bac507a0a2b77addc`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/layout`,
plus Mercury's changes in `third_party/patches/servo-layout.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-layout` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026).

### Caret in contenteditable (servo/servo#48253) — `flow/inline/text_run.rs`, `flow/inline/construct.rs`, `dom.rs`, `display_list/mod.rs`

- **What:** backport of servo/servo#48253 "Render caret inside of contenteditable elements" (merge commit `cd332167d`).
  `SharedTextRunData::paints_caret` is an `AtomicBool` (`text_run.rs`, set in `construct.rs` from
  `text_node_paints_caret`), written by `DOMLayoutData::set_text_run_selection(range, new_paints_caret)` (`dom.rs`) and read
  with `Ordering::Relaxed` where the caret rectangle is pushed (`display_list/mod.rs`, about line 1578), so the caret
  follows selection changes without rebuilding the text run. Needs the matching `servo-layout-api` and `servo-script`
  hunks.
- **Why:** before this only `<input>` and `<textarea>` drew a caret; Mercury turns on contenteditable editing.
- **Tested by:** `tests/pages/compat-editing.html` (screenshot with `--screenshot-delay 1600`: the caret blinks).
- **Drop when:** the Servo bump that contains PR #48253 (B1).

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-layout.patch` and this directory
once Mercury no longer needs to change the crate.
