# Patched servo-net-traits 0.8.0

This directory vendors `servo-net-traits` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`80e6ac24573425319af0ac9dbc3ec7eec925b9da656902a15815506fd5e3e417`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/shared/net`,
plus Mercury's changes in `third_party/patches/servo-net-traits.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-net-traits` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

None yet. For each change, say what it does, why Mercury needs it, the upstream
file and line it changes, how it is tested and when it can be dropped.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-net-traits.patch` and this directory
once Mercury no longer needs to change the crate.
