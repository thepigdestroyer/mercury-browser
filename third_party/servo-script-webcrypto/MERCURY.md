# Patched servo-script-webcrypto 0.8.0

This directory vendors `servo-script-webcrypto` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`c57ab02cc56817713f0a8a18a125c90604f66d20479a2cd6e572874b2b5c4d7b`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/script_webcrypto`,
plus Mercury's changes in `third_party/patches/servo-script-webcrypto.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-script-webcrypto` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026).

### SpiderMonkey 153.7.0 (`mozjs` 0.27.0) — `Cargo.toml`

- **What:** the `js` dependency (package `mozjs`) is `=0.27.0`, not `=0.26.13`, with a `# Mercury:` comment
  (servo/servo#49907, merge date 14 Nov 2026). `Cargo.lock` moves exactly `mozjs`, `mozjs_sys` (153.6.0-1 to 153.7.0-0) and
  the twelve `mozjs_*` glue crates (153.6.0 to 153.7.0). The same one-line change is in `servo-script` and
  `servo-script-bindings`.
- **Why:** SpiderMonkey 153.7.0 carries security fixes. `servo-script`, `servo-script-bindings` and this crate all pin
  `mozjs` exactly, so this crate had to be vendored (unchanged, in its own commit) to move the pin. Source files are
  unchanged.
- **Tested by:** `make deny` and the full build and test gates of lane K0.
- **Drop when:** the Servo bump that contains PR #49907 (B1). To go back to SpiderMonkey 153.6.0, revert the commit
  "build: SpiderMonkey 153.7.0 (mozjs 0.27.0) for its security fixes" and re-export.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-script-webcrypto.patch` and this directory
once Mercury no longer needs to change the crate.
