# Patched servo-constellation-traits 0.8.0

This directory vendors `servo-constellation-traits` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`a877d7852a4d254dc250fc6e9883b5c01db639012660aab65050be2f53835ec8`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/shared/constellation`,
plus Mercury's changes in `third_party/patches/servo-constellation-traits.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-constellation-traits` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026), part of the Mercury message scaffold (R14 section 3.4).

### Mercury messages between the script thread, the embedder and the constellation — `lib.rs`, `from_script_message.rs`, new `mercury/`

- **What:** `pub mod mercury;` and `pub use mercury::*;` in `lib.rs`; new variants
  `ScriptToConstellationMessage::Mercury(MercuryScriptToConstellation)` (`from_script_message.rs`, after
  `ReleaseWakeLock`; the enum is `Serialize + Deserialize`, so the script-side lane enums are too) and
  `EmbedderToConstellationMessage::Mercury(MercuryEmbedderToConstellation)` (`lib.rs`, after `SetNetworkOnlineState`).
  `mercury/` has, per lane `Core`, `A11y`, `Media`, `Page`, `Text`, one file with `<Lane>ScriptMsg` and
  `<Lane>EmbedderMsg`; both `CoreScriptMsg` and `CoreEmbedderMsg` have `Probe(u64)` (tests). The lane files are private
  modules whose public items are re-exported by glob; `mercury` itself is public so `servo` can re-export the lane types.
  `Debug` prints only the variant path, as in `servo-embedder-traits`.
- **Why:** a lane adds its messages in its own file and none edits the shared enums.
- **Tested by:** `cargo test --release -p servo-constellation-traits mercury` (3 tests).
- **Drop when:** never needed upstream; keep until Mercury stops patching Servo's message enums.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-constellation-traits.patch` and this directory
once Mercury no longer needs to change the crate.
