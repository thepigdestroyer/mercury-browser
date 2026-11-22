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

## Changes: page accessibility (lane a11y-1, 21 Nov 2026)

### `SUPPORTED_ARIA_ROLES` is no longer consulted — `accessibility.rs`

- **What:** a comment only. Layout no longer limits a node's role to the safe list; `mercury_a11y::roles` maps the whole
  ARIA role set. The statics stay for script's `computedRole`.
- **Drop when:** upstream servo#46241 lands.
Changes made by lane txt-1 (text, 21 Nov 2026).

### Typed highlights for text runs (EB7) — new `highlights.rs`, `lib.rs`, `layout_node.rs`

- **What:** new module `highlights.rs`: `HighlightKind` (`Composition`, `CompositionSelected`, `FindMatch`, `ActiveFindMatch`,
  `TextFragment`, `Spelling`, `Grammar`, `Custom(u32)`), `TextHighlight { kind, range }`, `TextHighlights = Vec<TextHighlight>`,
  `Decoration`/`DecorationSet`, `Segment` and `resolve_segments(run_range, highlights)`, which clips highlights to a text run,
  drops empty ones and splits overlaps: the winning background is `FindMatch < TextFragment < ActiveFindMatch` (`Custom` ranks
  below all), decorations stack and paint in the order composition, selected composition, spelling, grammar. `lib.rs`: the module,
  its re-exports, `malloc_size_of_is_0!` for the two plain types, and the trait method
  `GenericLayoutDataTrait::set_text_run_highlights(&self, TextHighlights) -> bool` next to `set_text_run_selection`.
  `layout_node.rs`: `LayoutNode::text_node_highlights` next to `text_node_selection`, so layout can ask for the initial value
  of a run it builds.
- **Why:** layout paints one range per text run (the selection). IME composition is underlined now; find in page, text fragments
  and spell check need typed ranges that script owns and layout only paints.
- **Upstream:** none (the CSS Custom Highlight API is not implemented in 0.8).
- **Tested by:** 13 unit tests in `highlights.rs` and `servo-layout/tests/highlights.rs`.
- **Drop when:** never as a whole; reconcile if upstream adds highlights.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-layout-api.patch` and this directory
once Mercury no longer needs to change the crate.
