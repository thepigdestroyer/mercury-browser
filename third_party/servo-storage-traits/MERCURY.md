# Patched servo-storage-traits 0.8.0

This directory vendors `servo-storage-traits` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`e74add8ac3dcd4bd29c91f6f06e4aaeb9d047e903d74d3324d4dbfe8d3447640`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/shared/storage`,
plus Mercury's changes in `third_party/patches/servo-storage-traits.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-storage-traits` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

None yet. For each change, say what it does, why Mercury needs it, the upstream
file and line it changes, how it is tested and when it can be dropped.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-storage-traits.patch` and this directory
once Mercury no longer needs to change the crate.
