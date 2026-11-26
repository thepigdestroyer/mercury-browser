# Patched servo-script-traits 0.8.0

This directory vendors `servo-script-traits` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`6b56acb2276ef730d1a71c1e076b3a8003e7442ce92778847422f773c566d00f`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/shared/script`,
plus Mercury's changes in `third_party/patches/servo-script-traits.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-script-traits` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

### `ScriptThreadMessage::Mercury` and the lanes' payload types — `lib.rs`, new `mercury/`

- **What:** a `Mercury(MercuryScriptThreadMsg)` variant after `Reload` in `ScriptThreadMessage`, and the new `mercury`
  module (`mercury/mod.rs`, `mercury/page.rs`) with `MercuryScriptThreadMsg` and `PageThreadMsg`. `Debug` on the payloads
  prints only variant names.
- **Why:** the constellation sends the page lane's messages (`PermitUnload`, `StopLoading`, `ReloadBypassingCache`,
  `NavigationRefused`) to the script thread. One wrapper variant keeps the upstream enum change to a single line for all lanes.
- **Tested by:** `cargo check` of `servo-script` and the page lane's tests.
- **Drop when:** Mercury no longer needs script-thread messages, or the lane's messages move elsewhere.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-script-traits.patch` and this directory
once Mercury no longer needs to change the crate.
