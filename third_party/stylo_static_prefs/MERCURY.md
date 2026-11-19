# Patched stylo_static_prefs 0.23.0

This directory vendors `stylo_static_prefs` 0.23.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`8e06c605a1d428d159798ebfed3f81f3a145f1838872360ebb24ec6d85993fe1`), built from https://github.com/servo/stylo at commit
`2dca34592ced37c6aa01733e94206a767194cf9d`, path `stylo_static_prefs`,
plus Mercury's changes in `third_party/patches/stylo_static_prefs.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff stylo_static_prefs` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

None yet. For each change, say what it does, why Mercury needs it, the upstream
file and line it changes, how it is tested and when it can be dropped.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/stylo_static_prefs.patch` and this directory
once Mercury no longer needs to change the crate.
