---
title: Camera and microphone
description: How web pages get the camera and microphone in Mercury Browser, and how to test it with fake devices only.
---

# Camera and microphone

Web pages can ask for the camera and the microphone (`navigator.mediaDevices.getUserMedia`) on macOS. On other platforms the engine preference that exposes `navigator.mediaDevices` stays off, so the API does not exist there.

## How it is gated

- **Engine.** Servo 0.8.0 as published opens the devices without asking. Mercury carries a patched `servo-script` (`third_party/servo-script/MERCURY.md`):
  - `getUserMedia` asks the browser for "camera" and "microphone" before it opens anything, and is refused with `NotAllowedError` when the answer is no;
  - device names are hidden until permission is granted;
  - `MediaStreamTrack.stop()` releases a device;
  - every device is released when its page is unloaded, reloaded or closed;
  - frames from another origin than the top-level page may not use either device.
- **Preferences** (`mercury_preferences` in `crates/talaria-servo/src/runtime.rs`):
  - `dom_webrtc_enabled` exposes `navigator.mediaDevices`. It is on for macOS only.
  - `dom_webrtc_transceiver_enabled` exposes `RTCPeerConnection` and the rest of WebRTC (the vendored bindings moved WebRTC behind it). It is always off: Servo 0.8's WebRTC gives every local IP address to the page and cannot call modern sites.
- **macOS.** The device is opened by the content process that runs the page, which is the same executable as the browser.
  - `apps/mercury-desktop/resources/macos/Info.plist.in` carries `NSCameraUsageDescription` and `NSMicrophoneUsageDescription`. macOS ends a process that opens a device without them.
  - Both entitlements files grant `com.apple.security.device.camera` and `com.apple.security.device.audio-input`. The hardened runtime refuses access without them.
  - A test pins all four keys (`embedded_info_plist_describes_camera_and_microphone_use`, `scripts/macos/test-bundle.sh`).
- **A binary started from a terminal.** macOS charges camera and microphone access to the *terminal application*, not to Mercury. A development build run from a shell could therefore prompt for, or silently use, the terminal's permission.

## Never use real devices in automated runs

Automated checks, and agents working on the code, never open the real camera or microphone, never cause a macOS privacy prompt and never play sound. Use the fake devices.

## Fake devices

`--fake-media-devices` is a diagnostic option. It is not in `--help`. It gives pages a "Mercury fake camera" (GStreamer's `videotestsrc`, a moving ball) and a "Mercury fake microphone" (`audiotestsrc`, a sine wave) through a GStreamer device provider. Each content process registers it before Servo starts its media backend (`crates/talaria-servo/src/fake_media_devices.rs`). Servo's own capture code then finds the fakes as it would find real devices.

The option is refused (exit status 2) in three cases:

- the run is not automated: it needs `--screenshot` or `--background`;
- the `lib` directory beside the executable contains `libgstapplemedia.dylib` (the camera source) or `libgstosxaudio.dylib` (the microphone source and the audio output);
- that directory lacks `libgstvideotestsrc.dylib` and `libgstaudiotestsrc.dylib`.

A normal build therefore cannot use it. Each content process checks the directory again. It then also removes those two plugins and every other device provider from GStreamer's registry, in case the registry cache names one.

`make test-capture-macos` (`scripts/macos/test-capture.sh`), after `make build-release`:

1. copies `target/release/mercury-browser` into a temporary directory;
2. collects its GStreamer libraries with:
   - `MERCURY_GSTREAMER_EXCLUDE_PLUGINS="ogg applemedia osxaudio"`;
   - `MERCURY_GSTREAMER_EXTRA_PLUGINS="videotestsrc audiotestsrc"`. The collector accepts only those two extras, and never for an application bundle;
3. checks the copy;
4. checks that the normal build refuses the option;
5. runs `tests/pages/capture.html` from `scripts/serve-tests.py`.

The page writes its results into the document title, which the window-title log carries out (`RUST_LOG=winit::window=debug`). GStreamer's element log (`GST_DEBUG=GST_ELEMENT_FACTORY:4`) shows whether a test source was created, that is, whether the engine opened a device. Set `MERCURY_TEST_CAPTURE_KEEP=<directory>` to keep the screenshots and logs.

What the checks cover today:
- the fake devices are registered;
- `navigator.mediaDevices` exists and `RTCPeerConnection` does not;
- before permission, `enumerateDevices` lists one input of each kind without id or label;
- with no answer given (the engine's default is "deny"), `getUserMedia` is refused and no device source is created.
