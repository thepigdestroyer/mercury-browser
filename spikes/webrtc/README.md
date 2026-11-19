# WebRTC spike (lane sp-rtc)

Can Mercury ship Google's libwebrtc as its WebRTC engine under its own rules: commercial-safe licences, no
Python or other unchecked build tooling, no unpinned download, no real camera, microphone or screen in tests?
The spike answers that with two Rust binding routes over the same pinned prebuilt, and leaves behind what the
real WebRTC lane (R14 item EA7, "MED-2") needs. The findings and the go or no-go are in `research/R18-webrtc-spike.md`
in the project notes (outside the repository); this directory is the code.

Nothing here is part of the default build, `make`, the gates or the product. Each crate is its own cargo workspace
with its own `Cargo.lock` and `target/`: run cargo from the crate's directory, not with `-p` from the root.

| Directory | What it is | Runs where |
|---|---|---|
| `sink-api/` | `mercury-frame-sink`: the std-only contract between a capture stream and a sender (`MediaFrameSink`, `SinkSet`, `ClockMap`) | anywhere |
| `bridge/` | `mercury-rtc-bridge`: GStreamer capture streams with a tee and one `appsink` per frame sink, and `RemoteSource` (an `appsrc`) for the way back; no libwebrtc | this Mac (Intel or Apple Silicon) |
| `livekit/` | route L: LiveKit's `libwebrtc` 0.4.1 and `webrtc-sys` 0.3.54 over the pinned Shiguredo `m150` archive | compile and link on any Mac with `--target aarch64-apple-darwin`; tests on Apple Silicon only |
| `shiguredo/` | route S: `shiguredo_webrtc` 0.157.0 over the pinned `libwebrtc_c` archive | same |

The two route crates run the same tests `t01` to `t12` (`tests/calls.rs`, one `RESULT tNN pass|fail|skip detail`
line each; route L also has `t13`, the bridge microphone): two peers in one process, data channel, stats, audio,
video from the bridge, simulcast, teardown, the privacy policy of ICE candidates, codecs, coexistence with GStreamer,
threads and shutdown, and the fail-closed build. The libwebrtc prebuilts exist for macOS arm64 only, so those tests run in CI
(`.github/workflows/webrtc-spike.yml`, `macos-26`), not on an Intel development Mac.

## The prebuilts

`scripts/macos/webrtc.sh` pins the two archives by size and SHA-256 (`list`, `fetch`, `unpack`, `check`, `path`,
`env`, `inspect`), like `scripts/macos/gstreamer.sh`; `scripts/macos/test-webrtc.sh` tests it without a network.
Nothing is downloaded by a build script: each route crate fails closed unless `webrtc.sh env` exported the
verified directory (route L: `LK_CUSTOM_WEBRTC` defaults to the empty `livekit/no-prebuilt/`; route S: the patched
`build.rs` refuses without `MERCURY_WEBRTC_C_DIR`). LiveKit's own prebuilt libwebrtc is never used (it links LGPL
FFmpeg and OpenH264 and has no checksum).

## Third-party code

The upstream crates are not committed. `prepare.sh` in each route crate copies them from the cargo registry into a
git-ignored `vendor/` (checking the crate's SHA-256) and applies the `.patch` files; a forgotten `prepare.sh` stops
cargo. The patches are Apache-2.0 hunks of the upstream files (see `REUSE.toml` and `THIRD-PARTY-NOTICES.md`).
No Python file of any upstream crate is committed or run.

## Running

```sh
scripts/macos/test-webrtc.sh                         # the pin script, hermetic, 12 s
eval "$(scripts/macos/gstreamer.sh env)"             # once per shell, needs `make gstreamer-macos`
cd spikes/webrtc/bridge && cargo test --release -- --test-threads=2
# route crates: see their README.md (fetch and unpack the archive, prepare.sh, webrtc.sh env, cargo test on Apple Silicon)
```

Through the build guard: `MERCURY_SLOT_POOL=light .superpowers/tools/guarded.sh <worktree> '<command>'`.
Delete every `spikes/webrtc/*/target` and `*/vendor` when you are done, and the archives in
`~/Library/Caches/mercury-browser/webrtc` (about 2.4 GB unpacked).
