# Patched servo-webgl 0.8.0

This directory vendors `servo-webgl` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`28d10b20a6a3dd77133b91fbd0e7840c582faf379ee5b870abad9429d0863aa6`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/webgl`,
plus Mercury's changes in `third_party/patches/servo-webgl.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-webgl` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane K0 (engine base, 19 Nov 2026).

### Zero-initialise `texStorage2D` and `texStorage3D` storage — `webgl_thread.rs`

- **What:** backport of servo/servo#49436 (merge commit `c2c5117bf`), hand-ported. `WebGLCommand::TexStorage2D` and
  `TexStorage3D` (0.8.0 lines about 1866 and 1885) call the new `zero_initialize_texture`, which uploads zeros with
  `tex_sub_image_2d` and `_3d` for every level, cube face or layer, in bands of at most 1 MiB with the pixel unpack buffer
  unbound meanwhile (compressed formats are skipped, an upstream TODO). New `tex_bytes_per_pixel` and a
  `#[cfg(test)] mod tests` with two unit tests, appended after `FramebufferRebindingInfo` (the upstream hunk had context
  from servo/servo#49404, which is not vendored). The script half is in `servo-script` (`dom/webgl/webgltexture.rs`).
- **Why:** texture storage from `texStorage2D` and `texStorage3D` is uninitialised GPU memory that a page can read back.
- **Tested by:** `cargo test --release -p servo-webgl --lib` (needs no GL context; heavy, not in Mercury's CI) and the page
  `tests/pages/webgl2-textures.html`. Both ported tests were run in a scratch crate against the real `TexFormat` and
  `TexDataType` code of `servo-canvas-traits` 0.8.0 and pass; `zero_initialize_texture` is identical to current upstream.
- **Drop when:** the Servo bump that contains PR #49436. Not taken: #49404 and #49902 (they change `servo-canvas-traits`,
  which is not vendored; they arrive with B1).

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-webgl.patch` and this directory
once Mercury no longer needs to change the crate.
