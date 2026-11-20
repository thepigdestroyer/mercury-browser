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

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo.patch` and this directory
once Mercury no longer needs to change the crate.
