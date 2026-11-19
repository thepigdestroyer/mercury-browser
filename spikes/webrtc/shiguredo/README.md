# Route S: libwebrtc through Shiguredo's `shiguredo_webrtc`

Part of the sp-rtc spike (WebRTC on libwebrtc). A spike crate in its own cargo workspace; nothing here is in the default
build. Findings are in `research/R18-webrtc-spike.md` (notes, outside the repository).

The binding crate is `shiguredo_webrtc` 0.157.0 (Apache-2.0): a C wrapper (`webrtc_c`) over libwebrtc's classes with 1:1 Rust
types, no runtime dependencies. Its build script downloads `libwebrtc_c-macos_arm64.tar.gz` with `curl` and a checksum file
from the same server. Mercury does not allow that, so the crate is used through a patched copy:

- `prepare.sh` copies the crate from the cargo registry (SHA-256 of the `.crate` checked) to `vendor/shiguredo_webrtc` (git-ignored),
  deletes its Python files and applies `patches/shiguredo_webrtc-0.157.0.patch`. `Cargo.toml` points `[patch.crates-io]` at
  `vendor/`, so a forgotten `prepare.sh` stops cargo.
- The patch (8 lines in `build.rs`, one new file of 76 lines) makes the build script take `lib/libwebrtc_c.a` and `bindings.rs`
  from `MERCURY_WEBRTC_C_DIR` and refuse to run without it: no download, no CMake, no libclang, no source build.
- `scripts/macos/webrtc.sh env libwebrtc-c` exports that variable, only after it has re-hashed the pinned archive.

## Build and run

The prebuilt is Apple Silicon only. On an Intel Mac everything but running works.

```sh
./prepare.sh
eval "$(../../../scripts/macos/webrtc.sh env libwebrtc-c)"
eval "$(../../../scripts/macos/gstreamer.sh env)"           # the tests link the bridge crate (GStreamer)
PKG_CONFIG_ALLOW_CROSS=1 cargo build --release --tests --target aarch64-apple-darwin   # Intel Mac: links, cannot run
./run-tests.sh --release                                    # Apple Silicon: t01 to t12, one process each
```

`.cargo/config.toml` sets the deployment target to 14.0 (the repository root says 13.0; the table form with `force` cannot be
merged with the root's string, so the value is a plain string). Run cargo from this directory, build only through the disk guard.

## Files

| File | What |
| --- | --- |
| `Cargo.toml`, `Cargo.lock`, `build.rs`, `.cargo/config.toml` | the workspace, its lock, the rpath for GStreamer in tests, the deployment target |
| `prepare.sh`, `patches/` | the patched third-party crate (not committed) |
| `src/` | what Mercury would write on top of the crate: engine, peer, channel, virtual audio device, video, sink adapters |
| `tests/calls.rs` | t01 to t12 of the shared test list (`RESULT tNN pass\|fail\|skip detail`) |
| `check-fail-closed.sh` | t12: runs the patched build script in nine scenarios (works on any host) |
| `run-tests.sh` | runs each test in its own process so that a libwebrtc abort is one failing test |
| `examples/size_probe.rs` | the call path without GStreamer, for the linked-size measurement |

## Licences

- `shiguredo_webrtc` is Apache-2.0; it is neither committed nor vendored here (`vendor/` is git-ignored).
- `patches/shiguredo_webrtc-0.157.0.patch` modifies an Apache-2.0 file (`build.rs`: 8 added lines, 12 lines of context), so the
  patch file stays under Apache-2.0 for those hunks. The new file it adds, `mercury_prebuilt.rs`, is Mercury's own and carries
  its own PolyForm header inside the patch.
- Everything else in this directory is PolyForm-Noncommercial-1.0.0 (SPDX header in each file); `Cargo.lock`, `README.md` and the
  patch cannot carry a header and are listed in `REUSE.toml` by the integrator.
