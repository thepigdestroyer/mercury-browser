# Patched servo 0.8.0

This directory vendors `servo` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`58a502704ce2fee976f20ee6192c575a3a77dfe844431f83cd4946da6e524985`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/servo`,
plus Mercury's changes in `third_party/patches/servo.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026), part of the Mercury message scaffold (R14 section 3.4).

### Delegate hooks for the Mercury messages — `lib.rs`, `servo.rs`, `servo_delegate.rs`, `webview_delegate.rs`, new `mercury/mod.rs`

- **What:** `WebViewDelegate::notify_mercury_message(&self, WebView, MercuryEmbedderMsg)` and
  `ServoDelegate::notify_mercury_message(&self, MercuryEmbedderMsg)`, both defaulted no-ops. `servo.rs` has two arms,
  in `handle_embedder_message` (`EmbedderMsg::Mercury`) and `handle_constellation_embedder_message`
  (`ConstellationToEmbedderMsg::Mercury`), that look the web view up and call `mercury::deliver`. `mercury/mod.rs` (new) has
  `deliver` (the web view's delegate when the web view exists, else the `ServoDelegate`, with one `debug!` naming the
  variant path when the web view is gone), `Servo::send_mercury_message(MercuryEmbedderToConstellation)` (sends
  `EmbedderToConstellationMessage::Mercury`; an inherent `impl Servo` there, so `servo.rs` has no hook line for it) and a
  re-export of the constellation-side lane types. `lib.rs`: `#[path = "mercury/mod.rs"] mod mercury_delivery;` (not named `mercury`: `pub use embedder_traits::*` re-exports that crate's `mercury` module, which a private module of the same name hides) and `pub use crate::mercury_delivery::*;` (the embedder
  side, `MercuryEmbedderMsg` and `CoreMsg`, already arrives through `pub use embedder_traits::*`).
- **Why:** one path from the engine to Mercury's delegates and back for every lane; `ServoInner` is private to `servo.rs`,
  so `deliver` takes what it needs as arguments.
- **Tested by:** two unit tests of `deliver` in `mercury/mod.rs` (no web view; web view gone):
  they need a compiled `servo` crate and cargo refuses `-p servo` from the Mercury workspace (dev-dependencies), so they have not been run; `talaria-servo`'s `mercury_messages` tests cover the router behind the delegate hooks, not `deliver` itself; the first message sent by a lane is its end-to-end test.
- **Drop when:** never needed upstream; keep until Mercury stops patching Servo's message enums.

Changes made by lane a11y-1 (21 Nov 2026).

### A child document's tree is never grafted as the page — `webview.rs`

- **What:** `WebView::process_accessibility_tree_update` treats an update with a newer epoch as a new top-level document
  and grafts its tree as the page. An update whose epoch carries the child-document flag
  (`embedder_traits::top_level_epoch_of_child_document`) is passed to the delegate unchanged, or dropped when its
  top-level document is already replaced.
- **Why:** an iframe's first update can arrive before the page's own.
- **Tested by:** `crates/talaria-servo/tests/a11y_frames.rs` (ignored, needs a GL context).
- **Drop when:** upstream distinguishes child documents' updates.
Changes made by lane txt-1 (text, 21 Nov 2026).

### IME sequences and the state messages — `tests/mercury_ime.rs` (new), `tests/input_methods.rs`, `tests/common/mod.rs`, `tests/rustfmt.toml` (new)

- **What:** `tests/input_methods.rs` (Servo's own file) gets only the test of servo/servo#49400 (`test_commit_without_composition`).
  The rest is in the new `tests/mercury_ime.rs`, so that a Servo bump does not conflict with it: its own harness (a copy of
  `InputMethodTest` with a Servo that has `execCommand` on, helpers for marked text and commit) and 25 tests for Pinyin in
  `<input>`, `<textarea>` and `contenteditable`, Japanese clauses with a selected clause, Korean (marked text replaced in place),
  a dead key, cancel (`Commit("")` and `SetMarkedText("")`), `Dismissed`, `InsertText` with and without a replacement (a current
  sequence, a stale one, changed text, past the end, inside a surrogate pair), `beforeinput` `preventDefault` during a
  composition, a composition left behind by a focus change, and the state messages (on focus, typing, selection change, `None`
  on blur, no text for a password, a 4,096-unit window for a long text, no message for a reflow that changed nothing about the
  field, a document id on every state and on every "cleared" message, an iframe's field). `tests/common/mod.rs` records every
  `TextMsg::InputState` and `TextMsg::InputCleared` that arrives through `notify_mercury_message`. The tests use a
  `SoftwareRenderingContext` and no window. `tests/rustfmt.toml` is Servo's own rustfmt configuration, so that `cargo fmt --all`
  (which formats the test files through `crates/mercury-engine-tests`) leaves the upstream test files alone.
- **Why:** the engine side of EB11 had no test of its own.
- **Tested by (how they run):** `servo` has dev-dependencies and is not a workspace member, so cargo refuses `cargo test -p servo`,
  and `--manifest-path third_party/servo/Cargo.toml` would resolve the published, unpatched sibling crates. The workspace member
  `crates/mercury-engine-tests` has `[[test]]` targets that point at these files and builds them against the patched crates:
  **`make engine-tests`** (needs GStreamer; no window; `scripts/ci/engine-tests.sh` runs each test in a process of its own, because
  Servo keeps its options in a process-wide `OnceLock` and a second engine in a process panics with "Already initialized").
  `test = false` keeps them out of `make test`. Two hook lines in `tests/common/mod.rs`: the patched `servo-default-resources` does
  not register its resource reader (the browser registers its own), so the test binary registers Servo's default one
  (`submit_resource_reader!`; without it the engine panics with "No resource reader registered" on its constellation thread and
  the test hangs), and `ServoTest::spin` fails after two minutes instead of spinning forever.
- **Drop when:** the #49400 test at the bump that contains it; the rest stays.

Changes made by lane net-1 (21 Nov 2026).

### The net lane's channel, the rewrite answer and the policy re-export — `servo.rs`, `lib.rs`, `webview_delegate.rs`

- **What:** `handle_net_embedder_message` (0.8.0 line about 531) has one arm for `NetToEmbedderMsg::Mercury`, which calls K0's
  `mercury::deliver` (the net thread's channel was not part of the scaffold). `lib.rs` re-exports `net::net_policy::{NetPolicy,
  NetPolicyStats, SiteKey}` for Mercury. `webview_delegate.rs` has `WebResourceLoad::rewrite(self, WebResourceRewrite)` (EC37, the
  answer that changes the URL or headers and lets the load go on), the test literal of `WebResourceRequest` takes the three new
  fields, and there is one new test.
- **Why:** Mercury sets the network policy and receives the mixed-content report (EC5, EC17, EC37).
- **Tested by:** `webview_delegate`'s `test_web_resource_load_rewrite` (needs this crate's dev-dependencies, so it is run on a Servo
  upgrade, not in `make test`) and `talaria-servo`'s `net_messages` tests.
- **Drop when:** the scaffold is dropped and Servo has equivalents.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo.patch` and this directory
once Mercury no longer needs to change the crate.
