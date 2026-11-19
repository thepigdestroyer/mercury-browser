# Patched servo-constellation 0.8.0

This directory vendors `servo-constellation` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`327d28714f0dcbb946844c83995ca01926d103c2bcbf6c6797f3f09c7893b4ea`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/constellation`,
plus Mercury's changes in `third_party/patches/servo-constellation.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-constellation` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026), part of the Mercury message scaffold (R14 section 3.4).

### Router for the Mercury messages — `constellation.rs`, `embedder.rs`, `tracing.rs`, new `mercury/`

- **What:** `mercury/mod.rs` dispatches each message to one `impl Constellation` function per lane file
  (`mercury/{core,a11y,media,page,text}.rs`; `Core::Probe` only traces; the others are `match message {}` until a
  lane gives them variants). `constellation.rs` declares the module with `#[path = "mercury/mod.rs"] mod mercury;`
  (not in `lib.rs`) so that it is a child of `constellation` and the lane files can use `Constellation`'s private
  fields and methods without editing this file again, and gets two match arms: `handle_request_from_embedder` (after
  `SetNetworkOnlineState`) and `handle_request_from_script` (after `ReleaseWakeLock`). `embedder.rs`:
  `ConstellationToEmbedderMsg::Mercury(Option<WebViewId>, MercuryEmbedderMsg)`. `tracing.rs`: two `LogTarget` arms
  (`constellation<embedder@Mercury`, `constellation<script@Mercury`). `lib.rs` is unchanged.
- **Why:** lanes implement their handlers only in `mercury/<lane>.rs`.
- **Tested by:** the `talaria-servo` router tests and the integration build; no unit test lives in this crate.
- **Drop when:** never needed upstream; keep until Mercury stops patching Servo's message enums.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-constellation.patch` and this directory
once Mercury no longer needs to change the crate.
