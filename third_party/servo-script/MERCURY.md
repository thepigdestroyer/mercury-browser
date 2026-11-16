# Patched servo-script 0.8.0 (and the media-capture patch)

This directory vendors `servo-script` 0.8.0, Servo's DOM and script engine glue,
under its original MPL-2.0 license. It is the crate exactly as published on
crates.io (checksum
`b6c3e52be4165b3f98b992f384156b049d30b769004a96cf52903ea27cd32f9b`), built from
https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/script`.

The crate was published without a licence file. Its `Cargo.toml` declares
`license = "MPL-2.0"` and every source file carries the MPL-2.0 header; the
repository's MPL-2.0 `LICENSE` file applies to this directory.

It is the home of Mercury's engine patches. The first one, camera and
microphone capture, spans four vendored crates that were added together in
commit `ea292e9` (each byte-identical to its published `.crate`), so

    git diff ea292e9 -- third_party/servo-script third_party/servo-script-bindings \
        third_party/servo-media-streams third_party/servo-media-gstreamer \
        ':(exclude)third_party/*/MERCURY.md'

shows every change, and only the changes listed here and in the other three
directories' `MERCURY.md`. Every change is marked with a `Mercury:` comment
(`grep -rn "Mercury:" third_party/servo-script`). How to re-vendor on a Servo
upgrade: `docs/developer-guide/vendored-patches.md`. The design and the engine
facts behind it: `.superpowers/research/R8-media-capture.md`.

## Changes: camera and microphone (`getUserMedia`)

In Servo 0.8.0 `getUserMedia` opens the camera and microphone without asking
anyone, `enumerateDevices` returns device names without permission, a
cross-origin frame can ask in its embedder's name, and nothing ever releases a
device (there is no `MediaStreamTrack.stop()`). Mercury turns capture on only
with these changes.

### 1. Ask before opening a device — `dom/media/mediadevices.rs`

- **What:** `GetUserMedia` rejects `{audio: false, video: false}` with a
  `TypeError`, a document that is not fully active with `InvalidStateError`,
  and an insecure context with `NotAllowedError`. It then asks for
  "microphone" and "camera" through `request_permission_to_use_async` — both
  requests at once, so the embedder can show them together — and opens no
  device (not even the device-monitor probe) before every requested kind is
  granted. The first refusal rejects with `NotAllowedError`; a missing device
  after the grant rejects with `NotFoundError` and stops a source already
  opened for the other kind. The tracks are "capture tracks", registered with
  the document's `MediaDevices`.
- **Upstream:** `mediadevices.rs:49-72`.
- **Drop when:** Servo asks for permission in `getUserMedia`.

### 2. Expose device ids and labels only with permission — `dom/media/mediadevices.rs`

- **What:** `EnumerateDevices` gives ids and labels only for kinds whose
  permission is granted (camera: `videoinput`; microphone: `audioinput`,
  `audiooutput`). With neither granted nothing is probed and one entry with
  empty fields stands for each input kind.
- **Upstream:** `mediadevices.rs:75-111`; `servo-media-gstreamer`
  `device_monitor.rs:36-46` uses the device name as its id.
- **Drop when:** Servo implements "device information can be exposed".

### 3. Bare constraint numbers are ideal values — `dom/media/mediadevices.rs`

- **What:** `{width: 1280}` and `{frameRate: 30}` are ignored instead of being
  turned into exact caps that exclude every device without that mode; `exact`,
  `min` and `max` still apply.
- **Upstream:** `mediadevices.rs:128-160`.

### 4. `MediaStreamTrack.stop()` and `readyState` — `dom/media/mediastreamtrack.rs`

- **What:** `readyState` ("live"/"ended") and `stop()`. Stopping a capture
  track releases its source once no live clone uses it; clones of capture
  tracks are registered too. (WebIDL in `servo-script-bindings`.)
- **Upstream:** `mediastreamtrack.rs:57-75`.

### 5. Release devices when the document goes away — `dom/document/document.rs`, `dom/window/window.rs`, `dom/navigator/navigator.rs`

- **What:** the unloading document cleanup steps (navigation, reload, frames)
  and `Window::clear_js_runtime_common` (pipeline exit: tab closed or
  discarded) call `Window::stop_media_capture`, which ends every capture track,
  stops their sources and forgets the document's camera and microphone
  answers, so a page restored from the history asks again. New accessors
  `Window::maybe_media_devices` and `Navigator::maybe_media_devices`.
- **Upstream:** `document.rs:871-901`, `window.rs:570-578`, `navigator.rs:187`.

### 6. Camera and microphone are `'self'` in frames — `dom/document/document.rs`

- **What:** `Document::allowed_to_use_feature` returns false for `Camera` and
  `Microphone` unless the document is the top-level one or same origin with it
  (the default allowlist of those policy-controlled features). Permission
  queries, requests and `getUserMedia` in a cross-origin frame are denied
  without a prompt. `allow=` on an iframe is not honoured. Servo labels every
  permission request with the top-level origin, so this also keeps a
  cross-origin frame from asking in its embedder's name.
- **Upstream:** `document.rs:3608-3625`.
- **Drop when:** Servo implements Permissions Policy and reports the asking frame.

## Warnings

Cargo does not cap lints for path dependencies, so compiling this crate in
Mercury shows rustc warnings that crates.io builds hide. With Mercury's feature
set (no `gamepad`, `webgpu`, testing hooks) and toolchain, the crate as
published gives 16; each is silenced at its own site, so that a new warning —
from a Mercury change or a Servo upgrade — still shows:

| Site | Warning | Silenced with |
|---|---|---|
| `dom/document/document_event_handler.rs` `use crate::dom::types::{…}` | unused import `GlobalScope` | `#[allow(unused_imports)]` on that `use` |
| `dom/html/embedded_content/htmlvideoelement.rs` `use script_bindings::error::Fallible` | unused import (used with `webgpu`) | `#[allow(unused_imports)]` |
| `dom/document/document.rs` `queue_document_completion` | unused variable `cx` | `#[allow(unused_variables)]` on the function |
| `tasks/task_manager.rs` `gamepad_task_source` | never used (only `gamepad` uses it) | `#[cfg(feature = "gamepad")]` on the definition |
| `dom/animations/animationtimeline.rs`, `documenttimeline.rs` `advance_specific`; `dom/bindings/weakref.rs` `MutableWeakRef::set`; `dom/canvas/imagedata.rs` `get_snapshot`; `dom/document/document.rs` `advance_animation_timeline_for_testing`; `dom/html/embedded_content/htmlcanvaselement.rs` `get_or_init_webgpu_context`, `is_valid`; `dom/window/window.rs` `gc`, `advance_animation_clock`; `dom/worklet/worklet.rs` `perform_a_worklet_task`; `event_loop/script_thread.rs` `handle_tick_all_animations_for_testing` | never used | `#[allow(dead_code)]` on the item |
| `dom/mod.rs` `mod types`; `dom/bindings/mod.rs` `mod Bindings` | unfulfilled `#[expect(unused)]` / `#[expect(non_camel_case_types)]` | `allow` instead of `expect` |

Cargo also reports `backtrace` as an unused dependency of the crate (it is used
only with `webgl_backtrace` and the testing hooks); `[lints.cargo]
unused_dependencies = "allow"` at the end of `Cargo.toml` silences that one
lint, which concerns only this manifest's dependency list.

The other three vendored crates compile without warnings.

## Removing this directory

Remove the `[patch.crates-io]` entry and this directory once a Servo release
contains equivalents of every change listed here and in the other vendored
crates' `MERCURY.md`, or once Mercury no longer needs them.
