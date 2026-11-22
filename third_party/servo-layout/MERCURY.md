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
Changes made by lane txt-1 (text, 21 Nov 2026): typed highlights (EB7), ordered lists and counters (ED5), `text-overflow` (ED7).

All test files named below are run by the workspace member `crates/mercury-engine-tests` (its `[[test]]` targets point at them);
`rustfmt.toml` (new, Servo's own settings) keeps `cargo fmt --all` from rewriting them.

### Typed highlights (EB7) — new `highlights.rs`, `flow/inline/text_run.rs`, `flow/inline/construct.rs`, `dom.rs`, `display_list/mod.rs`, `lib.rs`, `tests/highlights.rs`

- **What:** the arithmetic of painting typed highlights (`highlights.rs`: the UA colours, mapping DOM offsets to the laid-out
  text through the offset map, the `::first-letter` windowing, placing segments on glyphs widened to whole clusters, decoration
  rectangles, device-pixel snapping); `SharedTextRunData::highlights` beside `selection` (`text_run.rs`, the field only);
  `DOMLayoutData::set_text_run_highlights` with the same `::first-letter` guard as the selection (`dom.rs`); the initial value
  of a run from `LayoutNode::text_node_highlights` (`construct.rs`: `push_text` keeps its signature and delegates to
  `push_text_with_highlights`, so `construct_modern.rs` is untouched); `placed_text_highlights` and
  `build_display_list_for_text_highlights` in `display_list/mod.rs`, called before `build_display_list_for_text_selection`
  (backgrounds: find `#FFFF00`, active find `#FF9632`, text fragment `#FFE500`, under the glyphs and the selection rectangle) and
  after the glyphs (decorations: composition 1 CSS px underline, selected clause 2 px, in the text colour; spelling and grammar
  waves through `push_line`). The selection path (`selection`, `paints_caret`, `build_display_list_for_text_selection`) is unchanged.
  `lib.rs`: `pub mod highlights;` (public for `tests/highlights.rs`).
- **Why:** composition underlines now, and a base for find in page, text fragments and spell check.
- **Tested by:** `tests/highlights.rs` (16 pure tests: overlaps, clipping, astral characters, `text-transform` offset maps),
  run by `make test`. Painting was not seen in a window.
- **Limits:** RTL and vertical text are placed as the selection code places them (glyphs walked left to right); a `Custom`
  highlight has no colour yet.
- **Drop when:** never as a whole.

### Selection on replaced elements (servo/servo#49613) — `replaced.rs`

- **What:** backport of servo/servo#49613 (merge commit `af5f54ba7`, sabbCodes, 14 Nov 2026): `ReplacedContentKind::paints_selection_highlight`
  (everything except audio, iframe and canvas), used by `ReplacedContents::new` and `set_selection`. Needs the `servo-script` hunks.
- **Why:** a collapsed selection no longer tints an image; a canvas never does.
- **Drop when:** the Servo bump that contains #49613 (B1).

### UA widgets are not selected (servo/servo#49885) — `stylesheets/user-agent.css`

- **What:** hand-port of servo/servo#49885 (merge commit `afc9224b1`, Martin Robinson, 13 Nov 2026): `input[type=file]` joins the
  `user-select: none` list. (The published diff did not apply: K0's marker comment sits between its import lines.) Needs the
  `servo-script` traversal hunk.
- **Drop when:** the Servo bump that contains #49885 (B1).

### Ordered lists and CSS counters (ED5) — new `counters.rs`, `counter_walk.rs`, `counter_styles.rs`; `lists.rs`, `dom_traversal.rs`, `context.rs`, `layout_impl.rs`, `lib.rs`, tests

- **What:** a counter pass computes the value of every CSS counter once per layout that needs it. `counters.rs` is the algorithm
  (CSS Lists 3 scopes, `reversed()` starts, the implicit `list-item` counter, HTML `<ol start reversed>`, `<ul>`/`<menu>`/`<dir>`
  and `<li value>`) on plain input with no Servo type. `counter_walk.rs` (new: the walker, `CounterState`, `update_counters`; `lists.rs` keeps only the hooks `make_marker`, `marker_string` and `generate_counter_representation`) walks the styled DOM in box-construction order (element, `::marker`
  of a numbered item, `::before`, flat-tree children, `::after`), feeds the pass, and keeps the last table in
  `LayoutThread::counters` and `LayoutContext::counter_table`. `make_marker` reads the number of its list item and
  `generate_pseudo_element_content` reads `counter()` and `counters()`. `counter_styles.rs` writes a value in every predefined
  counter style Stylo accepts (55; numeric, alphabetic, symbolic, cyclic, fixed, additive, the longhand CJK styles, Ethiopic) with
  range, `pad`, `negative`, suffix and the fallback to `decimal`/`cjk-decimal`. **The hook** (`layout_impl.rs`, `Mercury: ED5` at each
  of five hunks, 24 lines): `update_counters` runs after the style traversal and before box construction; it computes the table on
  the first layout, or when a restyle left `Relayout` or box damage and either the last table had something that shows a counter
  or the damaged elements have to do with counters. When a place that the old and the new table both know shows another number,
  every box is rebuilt (`box_tree = None`, `Relayout` damage from the environment, the path of a device change), because box
  damage only rebuilds elements whose style changed. `layout.css.counters.enabled=true` is in Mercury's `STYLO_PREFS`.
- **Why:** every `<ol>` showed "0." and `counter()` showed 0 (`lists.rs`: "assume a value of 0").
- **Upstream:** servo/servo#30365 and #48194 ("layout: Give list items their ordinal number.", open on 20 Nov 2026; four of 55 styles,
  no named counters). Nothing from #48194 is used. `counter-set` is `engine = "gecko"` in Stylo 0.23 (ED6); `@counter-style` is ED31.
  `counters.rs` and `counter_styles.rs` are new MPL-2.0 files written from CSS Lists 3 and CSS Counter Styles 3 with Gecko's
  `nsCounterManager.cpp` and `CounterStyleManager.cpp` translated where it saved work (named at the top of each, and in
  `THIRD-PARTY-NOTICES.md`). No Ladybird code is copied.
- **Tested by:** `tests/counter_styles.rs` (29) and `tests/counters.rs` (44 and one ignored measurement), both engine-free, run by
  `make test` through the workspace member `crates/mercury-engine-tests` (`servo-layout` has dev-dependencies and is not a member);
  `tests/pages/compat-lists.html` for the engine (27 cases). The pass over a 10,000-item `<ol>` takes 2.1 ms (213 ns per item;
  7.1 ms with `counters()` in every marker), measured on prepared input on this Mac.
- **Gap:** `<ol start|reversed>` and `<li value>` changed by script after load do not re-number (no `attribute_mutated` in
  `htmlolistelement.rs`/`htmllielement.rs` dirties the node); a change of `counter-reset`/`counter-increment` by style does.
- **Drop when:** a Servo bump contains upstream counters (follow #30365): drop `counters.rs`, `counter_styles.rs`, the adapter in
  `lists.rs` and the hook; keep `compat-lists.html`.

### `text-overflow: ellipsis | <string>` (servo/servo#42394, hand-ported) — new `ellipsis.rs`; `fragment_tree/fragment.rs`, `flow/inline/line.rs`, `flow/inline/mod.rs`, `flow/inline/text_run.rs`, `display_list/{mod,hit_test,stacking_context,paint_traversal}.rs`, `query.rs`, `lib.rs`

- **What:** hand-port of the layout half of servo/servo#42394 "text-overflow: ellipsis" (RichardTjokroutomo, open since
  March 2026) onto 0.8's layout; the published diff failed on every layout file, and its Stylo half is not needed (see
  `engine_prefs.rs`). A line that overflows a block with `overflow-x: hidden | clip` and `text-overflow: ellipsis | "<string>"`
  (end side) has its text cut at a cluster boundary and a marker placed behind what is left.
  `InlineFormattingContextLayout::line_elision` (`flow/inline/mod.rs`) decides per line from the length the line breaker
  measured; `LineItemLayout::elide_text_run` (`flow/inline/line.rs`) wraps each text run of such a line in the new
  `Fragment::ElidedText` (the whole text fragment plus the number of kept glyphs) and, at the first run that is cut, places the
  marker (an anonymous `TextFragment` shaped with the block's font, the last child of the line). `display_list/mod.rs` paints only
  the kept glyphs, `hit_test.rs` hits only them (the pull request's TODO), `paint_traversal.rs` gets the arms and
  `PaintTraversalHandler::visit_elided_text` (default: the whole text). `ellipsis.rs` is the pure arithmetic (`cut_point`,
  `keep_within`, `group_clusters`, `end_side`; only `Au` in its signatures); `end_side` translates the end-side choice of Gecko's
  `layout/generic/TextOverflow.cpp` (MPL-2.0). `layout.css.text-overflow.enabled=true` is in Mercury's `STYLO_PREFS`.
- **Differences from the pull request (on purpose):** positions are measured in line coordinates with the padding, border and
  margin of every open inline box counted; the cut never splits a glyph cluster; the condition is `overflow-x: hidden | clip` in
  a horizontal writing mode (not "not visible" and `display: flow`); `<string>` values work and three full stops stand in when the
  font has no U+2026; RTL elides the left end; hit tests skip elided text; `scrollWidth` of an elided fragment is the whole text.
- **Why:** truncated titles on YouTube, Gmail, GitHub and Reddit run on instead (R14d ED7, P0).
- **Known limits:** atomic inlines are never hidden; `overflow: auto | scroll` and vertical writing modes are not elided; the
  second value of the two-value syntax is parsed and ignored; the marker is not clipped to the content box.
- **Grapheme clusters:** the cut never falls inside a user-perceived character. `ellipsis::group_clusters_in_text` takes the text
  of the fragment (read from the inline formatting context's text, `line.rs::text_of_fragment`) and keeps the glyphs of one
  extended grapheme cluster together (UAX 29 through the `unicode-segmentation` crate, a new dependency of `servo-layout`: MIT OR
  Apache-2.0, already in the lock through Servo's other crates): an emoji ZWJ sequence, a flag, an emoji with a skin-tone modifier,
  an Indic conjunct, a decomposed Hangul syllable, CR LF, and a ligature over several graphemes. The glyph rule (a glyph with no
  character or no advance joins the one before) stays as the fallback and as a second reason to join; if the characters of the
  text and of the glyphs disagree, the text is not used.
- **Tested by:** `tests/ellipsis.rs` (24 pure tests, run by `make test` through `crates/mercury-engine-tests`),
  `tests/pages/compat-ellipsis.html` (14 rows).
- **Drop when:** the Servo bump that contains #42394 (or its successor): restore `fragment.rs`, `line.rs`, `inline/mod.rs`,
  `display_list/{mod,hit_test,stacking_context,paint_traversal}.rs`, `query.rs`, `text_run.rs`; delete `ellipsis.rs` and its test.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-layout.patch` and this directory
once Mercury no longer needs to change the crate.
