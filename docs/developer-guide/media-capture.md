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

## The browser side

- **Prompt.** `PagePermission::is_promptable` includes `Camera` and `Microphone`.
  - A camera and a microphone request from the same page waiting at the same time are shown as one prompt, "wants to use your camera and microphone", and answered together. The engine sends both requests at once for this. See `PendingPageRequest::paired_permission` in `talaria-core`.
  - The text comes from `apps/mercury-desktop/src/media_capture/`. The bubble is the dialogs lane's `PermissionPrompt`, with a message line and a checkbox added.
- **Remember for this site until you quit.** Offered only for camera and microphone, because the engine refuses both to frames of another origin, so the origin shown is the one asking.
  - It is kept per origin (scheme, host, port) and per device kind, in the session's `PermissionDecisions` store. A Block can be remembered too.
  - Pages without an origin (`file:`) are not offered it.
- **Indicator.** `BrowserController::capture_access(tab)` says what the tab's current document has been allowed to use.
  - It is set by an Allow: the user's, a remembered one, or another tab's remembered answer to a waiting request.
  - It is cleared when a new document starts, when the page crashes, when the web view goes away, and on Stop.
  - The tab shows a red mark (`CaptureTabMark`). A red button sits beside the address bar (`CaptureButton`); it opens `CaptureBubble` with Close and Stop. Colours: `ChromePalette.capture-indicator` and `capture-indicator-glyph`.
- **Stop** (`BrowserController::stop_capture`) forgets the origin's remembered camera and microphone answers, clears the indicator and reloads the tab. The engine's unload releases the devices.
  - Servo 0.8 cannot be told to stop capturing without reloading, and does not say when capture starts or ends.
  - A live indicator and Stop without reload need a second engine patch; see R8, section 10.
- **macOS refusal.** In an interactive run, `media_capture::system_access` asks AVFoundation (`authorizationStatusForMediaType:`; the framework is loaded only then) whether Mercury may use the device.
  - "Denied" and "restricted" turn the prompt into a notice with **Open Privacy Settings** (`x-apple.systempreferences:` URL) and **Close**; the page is refused.
  - "Not determined" shows the normal prompt. macOS then asks the user itself when the content process opens the device.
  - Automated runs never ask macOS: `--screenshot` and `--background` assume "allowed".

## Never use real devices in automated runs

Automated checks, and agents working on the code, never open the real camera or microphone, never cause a macOS privacy prompt and never play sound. Use the fake devices.

## Fake devices

`--fake-media-devices` is a diagnostic option. It is not in `--help`. It gives pages a "Mercury fake camera" (GStreamer's `videotestsrc`, a moving ball) and a "Mercury fake microphone" (`audiotestsrc`, a sine wave) through a GStreamer device provider. Each content process registers it before Servo starts its media backend (`crates/talaria-servo/src/fake_media_devices.rs`). Servo's own capture code then finds the fakes as it would find real devices.

The option is refused (exit status 2) in three cases:

- the run is not automated: it needs `--screenshot` or `--background`;
- the `lib` directory beside the executable contains `libgstapplemedia.dylib` (the camera source) or `libgstosxaudio.dylib` (the microphone source and the audio output);
- that directory lacks `libgstvideotestsrc.dylib` and `libgstaudiotestsrc.dylib`.

A normal build therefore cannot use it. Each content process checks the directory again. It then also removes those two plugins and every other device provider from GStreamer's registry, in case the registry cache names one.

`make test-capture-macos` (`scripts/macos/test-capture.sh`), after `make build-release`. The release executable finds GStreamer only in `target/release/lib`, which `make build-release` fills; the script stops with a message if the directory has no GStreamer libraries, and CI fills it with `scripts/macos/collect-gstreamer.sh` before it runs the script. The script:

1. copies `target/release/mercury-browser` into a temporary directory;
2. collects its GStreamer libraries with:
   - `MERCURY_GSTREAMER_EXCLUDE_PLUGINS="ogg applemedia osxaudio"`;
   - `MERCURY_GSTREAMER_EXTRA_PLUGINS="videotestsrc audiotestsrc"`. The collector accepts only those two extras, and never for an application bundle;
3. checks the copy;
4. checks that the normal build refuses the option;
5. runs `tests/pages/capture.html` from `scripts/serve-tests.sh`.

The page writes its results into the document title, which the window-title log carries out (`RUST_LOG=winit::window=debug`). The fake devices write a line to standard error when the engine opens one, and GStreamer's `GST_PARENTAGE` log shows a source taken out of its pipeline (released). Set `MERCURY_TEST_CAPTURE_KEEP=<directory>` to keep the screenshots and logs.

`--answer-permission-prompts none|allow|deny` (only with `--fake-media-devices`) answers every permission prompt once it has been on screen for 0.6 s. Allow answers as a click on the main button would, but never opens System Settings. `--fake-system-denies-capture` (only with `--fake-media-devices`) behaves as if macOS refused both devices. Neither option is in `--help`.

The runs, each a screenshot of `tests/pages/capture.html` or `capture-frame.html` served on loopback:

| Run | Checks |
| --- | --- |
| unanswered | the fakes are registered; the content process's log is captured (control); `navigator.mediaDevices` exists and `RTCPeerConnection`/`RTCIceCandidate` do not; before permission `enumerateDevices` gives one input of each kind without id or label; `getUserMedia` waits, and no device is opened before an answer. The screenshot shows the combined prompt |
| allow | one prompt answers both kinds; the engine opens the fake camera and microphone once each; a video and an audio track; device names after permission; the camera's frames move; `stop()` ends both tracks and takes the source out of its pipeline before the browser shuts down |
| deny | `NotAllowedError`; nothing opened |
| leave | the page navigates away with its devices open; the source is released by the unload, before shutdown |
| cross-frame | a frame from `localhost` in a `127.0.0.1` page: `permissions.query` says "denied", `getUserMedia` gives `NotAllowedError`, and no prompt is shown |
| same-frame | a frame of the page's own origin asks and gets the devices |
| system-denied | with macOS refusing, an automatic Allow still refuses the page and opens nothing; a second run without an answer shows the notice in its screenshot |

"Before the browser shuts down" is read from the order of the log lines. Both processes write to the same log, and the browser releases everything at exit anyway, so the release has to come before the line about the saved screenshot.

Verified only by unit tests and component renderings, because they need a click:
- ticking "Remember", whose effect is covered by `crates/talaria-core/tests/media_capture.rs`;
- the capture button, its bubble and Stop (`crates/mercury-ui/tests/capture_render.rs`, and `stop_capture` in the core tests);
- **Open Privacy Settings**.

## The owner's checklist (real devices; never automated)

Run a release bundle (`make app-macos`) opened from Finder, so that macOS attributes the devices to Mercury Browser and not to a terminal. Use a camera test page on a site you trust, or `tests/pages/capture.html` from `scripts/serve-tests.sh`. Note that this page plays nothing aloud; it never attaches the microphone to an output.

1. The page asks: one bubble for camera and microphone. Block: the page reports `NotAllowedError`, and macOS's camera light stays off.
2. Reload, Allow: macOS asks for the camera (the first time) and then the microphone. Allow both. The video shows you, and the camera light and the menu-bar dot come on. Tab and address bar show the red marks.
3. Click the red button, then Stop: the page reloads, the camera light goes off, and the menu-bar dot disappears within a few seconds. The page asks again.
4. Allow with "Remember for this site until you quit", then reload: no prompt. Quit and reopen: asked again.
5. Close a tab whose page uses the camera: the light goes off.
6. In System Settings, Privacy & Security, Camera, turn Mercury Browser off. The page's next request shows the "macOS does not allow" notice, and Open Privacy Settings opens that pane. Turn it back on.
7. A page in a frame of another site asking for the camera: refused without a prompt.
