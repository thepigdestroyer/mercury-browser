# Patched servo-media-player 0.8.0

This directory vendors `servo-media-player` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`fd072c0c9456f27cd17e9b3a752bf3a6e570468e806bc7d085d1a521e76f9083`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/media/player`,
plus Mercury's changes in `third_party/patches/servo-media-player.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-media-player` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

1. **`lib.rs`: `StreamType::MediaSource` and `Player::connect_media_source`** (lane med-1, MSE by Plan B).
   A third stream type, for a player whose data comes from a MediaSource: it pulls coded frames from the
   source's feed instead of being pushed bytes (`push_data` is never used for it). `connect_media_source`
   hands the player its `Arc<dyn MediaSourceBackend>` and must be the first call; its default body refuses,
   so other backends compile unchanged.
2. **`lib.rs`: `pub use mercury_mse as mse`**, and **`Cargo.toml`: a path dependency on `crates/mercury-mse`**.
   The MSE contract (`MediaSourceBackend`, `MseEvent`, `MseError`, `FrameFeed`, ...) is Mercury's own Servo-free
   crate; the player crate re-exports it so servo-media, servo-media-gstreamer and servo-script name it through
   a crate they already depend on. A vendored crate depending on a workspace member is the direction the
   vendoring rules allow (`make servo-patches-check` and `cargo fmt --all` accept it). Not in upstream.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-media-player.patch` and this directory
once Mercury no longer needs to change the crate.
