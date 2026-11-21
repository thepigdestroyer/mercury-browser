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

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-embedder-traits.patch` and this directory
once Mercury no longer needs to change the crate.
