# Patched servo-media 0.8.0

This directory vendors `servo-media` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`710187bb2f163fcf0cbb30a2c40684ff1bd5ab3a4ffbe7d338a099f4c6bf2b2f`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/media/servo-media`,
plus Mercury's changes in `third_party/patches/servo-media.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-media` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

1. **`lib.rs`: `Backend::create_media_source` and `Backend::is_type_supported_mse`** (lane med-1, MSE by Plan B).
   Two new trait methods with default bodies ("not supported", `false`), so a backend that knows nothing of
   Media Source Extensions (the dummy one) compiles unchanged. `create_media_source` takes a
   `GenericCallback<MseEvent>` and returns an `Arc<dyn MediaSourceBackend>`: the script thread makes the callback
   (it queues a media-element task), the backend sends its events from its own worker thread. The types are
   Mercury's (`crates/mercury-mse`, re-exported as `servo_media_player::mse`). Not in upstream; stays while
   Mercury carries its own MSE (the drop condition is in `third_party/servo-script/MERCURY.md`).

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-media.patch` and this directory
once Mercury no longer needs to change the crate.
