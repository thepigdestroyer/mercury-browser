# Patched servo-media-webrtc 0.8.0

This directory vendors `servo-media-webrtc` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`8ea6364262fade2feba53d33baced386299ad3aca0745af4cbb3a47e3613981f`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/media/webrtc`,
plus Mercury's changes in `third_party/patches/servo-media-webrtc.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-media-webrtc` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

None yet. For each change, say what it does, why Mercury needs it, the upstream
file and line it changes, how it is tested and when it can be dropped.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-media-webrtc.patch` and this directory
once Mercury no longer needs to change the crate.
