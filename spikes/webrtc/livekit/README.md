# WebRTC spike, route L: LiveKit's bindings over the pinned Shiguredo libwebrtc

`mercury-rtc-livekit-spike` links Google's libwebrtc (the Shiguredo `webrtc-build` m150 prebuilt, pinned by
SHA-256 in `scripts/macos/webrtc.sh`) through LiveKit's Rust crates `libwebrtc` 0.4.1 and `webrtc-sys` 0.3.54.
It is a spike, its own cargo workspace, and is not part of the default build. The archive is arm64 macOS only:
an Intel Mac compiles and links it with `--target aarch64-apple-darwin` and cannot run it; the tests run in CI
(`.github/workflows/webrtc-spike.yml`, job `livekit`).

LiveKit's own prebuilt libwebrtc is never used or downloaded (`webrtc-sys-build-local` has no downloader).

## Build

```sh
cd spikes/webrtc/livekit
../../../scripts/macos/webrtc.sh fetch unpack libwebrtc   # once (pinned archive)
./prepare.sh                                              # vendor/ = webrtc-sys + libwebrtc from the registry (SHA-256 checked, patched),
                                                          # and the pinned libc++ headers (fetch-libcxx.sh)
eval "$(../../../scripts/macos/webrtc.sh env libwebrtc)"  # LK_CUSTOM_WEBRTC
eval "$(../../../scripts/macos/gstreamer.sh env)"         # the bridge crate (tests t05, t07, t10, t13)
cargo test --release -- --nocapture --test-threads=1      # on Apple Silicon
cargo build --release --target aarch64-apple-darwin --tests --examples    # from an Intel Mac: compile and link only
```

Without the `webrtc.sh env` line the build stops: `.cargo/config.toml` points `LK_CUSTOM_WEBRTC` at the empty
`no-prebuilt/`, and `webrtc-sys-build-local` refuses a directory without `lib/libwebrtc.a` and the
`.mercury-webrtc-verified` marker. Without `prepare.sh`, cargo cannot find `vendor/`.

## What is here

- `patches/webrtc-sys-0.3.54.patch`, `patches/libwebrtc-0.4.1.patch`: the changes to LiveKit's crates (22 and 3 changed lines).
- `overlay/webrtc-sys/`: new files copied into the patched crate (`mercury_libcxx.rs`, `mercury_compat.h`).
- `webrtc-sys-build-local/`: replacement for `webrtc-sys-build` 0.3.21; `macos-arm64.defines` lists the preprocessor
  defines with the source of each.
- `fetch-libcxx.sh`: the libc++ headers of the archive's revision (`std::__Cr`), pinned by content hash.
- `src/`: `LiveKitVideoSink` and `LiveKitAudioSink` (`MediaFrameSink` into `NativeVideoSource` and `NativeAudioSource`),
  and `loopback` (two peers in one process).
- `tests/calls.rs`: t01 to t13, each printing `RESULT tNN pass|fail|skip detail`.
- `examples/probe_route.rs`, `examples/probe_baseline.rs`: the size measurement.

The findings are in `research/R18-webrtc-spike.md` (notes outside the repository).

## Status (19 Nov 2026)

Compiled and linked for `aarch64-apple-darwin` from an Intel Mac (library, tests, two size probes; clippy with `-D warnings` and `cargo fmt` clean; a cold
offline build from an empty `target/` took 8 minutes on a loaded 12-thread Mac). Nothing has been run: t01 to t13 await the arm64 CI job. Route L passes the gates
that can be judged without running it; see `research/R18-parts/3-livekit.md` for the gate table and for what to look at in the CI output (t03, t05, t06, t10, t11).
