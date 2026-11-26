# Patched servo-embedder-traits 0.8.0

This directory vendors `servo-embedder-traits` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`76d97f10b2d5979cd0ff330ab257a69addcf08e853811f6f97ff12db5f7468af`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/shared/embedder`,
plus Mercury's changes in `third_party/patches/servo-embedder-traits.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-embedder-traits` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026), part of the Mercury message scaffold (R14 section 3.4); the same
scaffold is in `servo-constellation-traits`, `servo-constellation` and `servo`.

### One `Mercury` message variant and one file per lane — `lib.rs`, new `mercury/`

- **What:** `pub mod mercury;` and `pub use crate::mercury::*;` next to the other modules (`lib.rs` lines about 14 and 58)
  and a new variant `EmbedderMsg::Mercury(Option<WebViewId>, MercuryEmbedderMsg)` after `AccessibilityTreeUpdate` (line
  about 570). `mercury/mod.rs` defines `MercuryEmbedderMsg` with one sub-enum per Mercury lane (`Core`, `A11y`, `Media`,
  `Net`, `Page`, `Text`), each in its own file (`mercury/<lane>.rs`, a private module re-exported by glob). All are empty
  except `CoreMsg::Probe(u64)` (tests only). The `Debug` output of every type in the directory is the variant path only
  (`Core::Probe`), so a page's data cannot reach a log. `size_of::<EmbedderMsg>()` is 248 bytes in 0.8.0 and 248 with the
  variant (measured, 64-bit).
- **Why:** every wave-1 engine lane adds its messages in its own file; none edits a shared enum, `EmbedderMsg` or
  `servo.rs`. The lane files are private modules so that `core` and `net` do not shadow the real crates through the glob
  re-export.
- **Tested by:** `cargo test --release -p servo-embedder-traits mercury` (5 tests: `Send + 'static`, `Debug`, the wrapper names
  only itself, size) and `talaria-servo` `mercury_messages::tests` (serde round trip, wire format, size of `EmbedderMsg`).
- **Drop when:** never needed upstream; keep until Mercury stops patching Servo's message enums.

Changes made by lane a11y-1 (21 Nov 2026).

### The epoch of a child document's accessibility tree — `mercury/a11y.rs`

- **What:** `epoch_for_child_document` and `top_level_epoch_of_child_document`: a child (iframe) document's updates carry
  its top-level document's epoch with the top bit set, so `WebView` can tell them from a new top-level document without a
  new message.
- **Drop when:** the constellation change in `servo-constellation` is dropped.
Changes made by lane txt-1 (text, 21 Nov 2026): input methods (EB11).

### IME events: commit, marked text, insert text — `input_events.rs`

- **What:** `ImeEvent` gains `Commit(String)` (**backport of servo/servo#49400**, merge commit `13fbe153e`, 5 Nov 2026: "can be
  sent without an active composition") and Mercury's `SetMarkedText { text, selected, clauses, replacement }` and
  `InsertText { text, replacement }`, with `ImeClause`, `ImeClauseKind` (`Raw`, `Converted`, `SelectedRaw`,
  `SelectedConverted`) and `ImeReplacement { range, sequence }`; every range is in UTF-16 code units. `Composition(..)` and
  `Dismissed` keep their meaning, so Servo's own tests and servoshell are unaffected.
- **Why:** macOS gives the selected range, the clause attributes and a replacement range, and inserts text without a composition
  (emoji picker, dictation, accent menu); winit 0.30 and the old events could not carry them. Composition in text controls and in
  `contenteditable` (EB11) needs them.
- **`Debug`:** `ImeEvent` has no derived `Debug` (`input_events.rs`: the derive is removed). Upstream logs whole messages at `debug`
  level (`script_thread.rs`: "Handling event: {event:?}"), and a derived `Debug` prints the typed text, as `Commit`,
  `SetMarkedText`, `InsertText` and `Composition` carry it. `impl Debug for ImeEvent` (in `mercury/text.rs`) prints the variant and
  the length of its strings.
- **Tested by:** `servo/tests/mercury_ime.rs` (see `servo/MERCURY.md`; `make engine-tests`), and `mercury/text.rs` for `Debug`.
- **Drop when:** the `Commit` variant at the Servo bump that contains #49400 (B1); the rest stays (Mercury's feature).

### The text lane's message and the text input state — new `mercury/text.rs` (the lane's own file of the scaffold)

- **What:** `TextMsg::InputState(Box<TextInputState>)` (boxed: unboxed it would grow `EmbedderMsg`, and the size test in
  `mercury/mod.rs` forbids that), `TextMsg::InputCleared(u64)` (nothing editable is focused in that document) and
  `TextInputState { document, sequence, text, text_start, selection, marked, caret_rect, multiline, is_password }`. `document` is
  the document's pipeline id, on every state and on the "cleared" message: a page and an iframe each send their own messages,
  which reach the browser in either order when the focus moves between them, and a late "cleared" of the page must not hide the
  frame's field (the browser clears only the state of the document named). The state is the text of the focused editable as a window of at most 4,096 UTF-16 units around the marked text or the
  selection, never cutting a surrogate pair; **empty for a password field**. `TextWindow` (utf16/byte offset helpers, the window
  rule, a caret rectangle estimate). `Debug` of both prints sizes and flags only, so the text of a page cannot reach a log.
- **Why:** AppKit's accent menu, dictation corrections and reconversion ask the text field for its text and selection, and the
  candidate window needs the caret rectangle; the browser process has neither without this message.
- **Tested by:** 14 unit tests in `mercury/text.rs` (windowing, surrogate pairs, password, `Debug`, `same_content_as`).
- **Drop when:** never needed upstream.

Changes made by lane net-1 (frame identity, the rewrite hook and the mixed-content report).

### Frame identity and a rewrite message on `WebResourceRequest` (EC38, EC37) — `lib.rs`

- **What:** `WebResourceRequest` (0.8.0 line about 693) gains `top_level_site: Option<String>`, `initiator_origin:
  Option<String>` and `is_third_party: Option<bool>`; `WebResourceResponseMsg` gains the terminal variant
  `Rewrite(WebResourceRewrite)` (after `DoNotIntercept`, line about 733). `WebResourceRewrite { url, set_headers,
  remove_headers }` is defined in `mercury/net.rs` (Mercury's file; its `Debug` prints no URL and no header value). Two
  existing fields change meaning, with a comment each: `url` is the URL of the **current hop** (0.8.0 reported the first URL
  of the redirect chain, so a rewrite of a redirect hop was meaningless), and `is_for_main_frame` is true only for a
  navigation of the top-level frame (a frame navigated by a script or a link is also fetched with destination `Document`).
- **Why:** the content blocker needs to know whether a request is third party and which frame it belongs to without
  guessing from the web view's URL, and EC37 needs a point where a URL can be rewritten (`$removeparam`).
- **Tested by:** `servo-net/tests/mercury_request_path.rs` (the fields for first-party, third-party, main-frame, nested and
  worker requests; the rewrite round trip), `cargo test -p servo webview_delegate`, and `talaria-servo`'s `adblocker` tests.
- **Drop when:** Servo has frame identity and a rewrite point in `WebResourceRequest`.

### `NetMsg::MixedContentBlocked` — `mercury/net.rs`

- **What:** the net lane's file of the K0 scaffold now has `NetMsg::MixedContentBlocked { url, destination }` (its `Debug`
  prints the variant path only) and `WebResourceRewrite`.
- **Why:** EC17 reports a blocked mixed-content request to Mercury, which counts them per tab.
- **Tested by:** the unit tests in `mercury/net.rs`, `servo-net/tests/mercury_request_path.rs` and `talaria-servo`'s `net_messages`.
- **Drop when:** never needed upstream; keep with the scaffold.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-embedder-traits.patch` and this directory
once Mercury no longer needs to change the crate.
