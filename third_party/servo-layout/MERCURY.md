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
- **Tested by:** `tests/pages/compat-editing.html` in a normal, focused window (the caret box at the top shows a red blinking
  caret). **Not by a `--screenshot` run:** that is a background window, the WebView has no system focus and
  `display_list/mod.rs` (line about 1583) never paints a caret then; the page prints `document.hasFocus()` (false in a screenshot
  run, checked on 20 Nov: no caret at delays 400, 1600 and 2800 ms) so the missing caret is not mistaken for a failure. The
  blink itself has not been seen in K0: no screen capture was possible in the integration session.
- **Drop when:** the Servo bump that contains PR #48253 (B1).

## Changes: page accessibility (lane a11y-1, 21 Nov 2026)

### Roles, names, states, values, relations and iframes in the accessibility tree (EE2-EE4, EE7) — `accessibility/{accessibility_node,accessibility_tree,mod}.rs`, new `accessibility/{dom_adapter,semantics,frames}.rs`

- **What:** upstream gives a node one of 16 HTML roles (35 ARIA roles through a safe list), a name for headings and links
  only, and nothing else. `dom_adapter.rs` lets the engine-free crate `mercury-a11y` read the layout DOM
  (`a11y_dom::{Document, Node, Rendering}` and `mercury_a11y::HostState` over `ServoLayoutNode`);
  `semantics.rs` calls `mercury_a11y::compute` and copies the result onto the `AccessibilityNode` (keeping children,
  bounds, transform and the hidden flag), maps relations to AccessKit node ids (unresolved ones are retried, never a
  panic) and recomputes the nodes whose name depends on a node that changed; `frames.rs` gives an `<iframe>` node
  the `tree_id` of its document, so the node is the graft (decision D11). `accessibility_node.rs` and
  `accessibility_tree.rs` have hook lines tagged `Mercury:` (the old name-from-descendants code is removed; layout's
  `display: none` and the semantic hide are two flags).
- **Why:** VoiceOver needs roles, names, states and values to read a page, and iframes to be part of the tree.
- **Upstream:** `accessibility/accessibility_node.rs` `update_properties_from_dom_node`, `update_node_local`,
  `HTML_ELEMENT_ROLE_MAPPINGS`, `NAME_FROM_CONTENTS_ROLES` (0.8.0 lines 348-440, 770-866).
- **Tested by:** the pure logic by `crates/mercury-a11y` tests (light pool); the glue by the `headless_servo_*` tests in
  `crates/talaria-servo/tests/a11y_pages.rs` and `a11y_frames.rs`, which have not been run (they need a GL context).
- **Drop when:** upstream servo#46241 (roles) and its name, state and iframe follow-ups land.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-layout.patch` and this directory
once Mercury no longer needs to change the crate.
