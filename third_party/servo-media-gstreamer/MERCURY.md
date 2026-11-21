# Patched servo-media-gstreamer 0.8.0

This directory vendors `servo-media-gstreamer` 0.8.0, Servo's GStreamer media
backend, under its original MPL-2.0 license (declared in `Cargo.toml`; MPL-2.0
headers; no licence file in the crate). It is the crate exactly as published on
crates.io (checksum
`7bb5bc60a53436a224a57f23059a494bf84296119f1c6e8f5bbba8a2af3fa96f`), built from
https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path
`components/media/backends/gstreamer`. `third_party/patches/servo-media-gstreamer.patch` is the difference from the
published crate, and `make servo-patches-check` fails if the two disagree.

It is part of the media-capture patch described in
`third_party/servo-script/MERCURY.md`.

## Changes

1. **`media_stream.rs`:** `GStreamerMediaStream::stop()` sets the stream's
   pipeline to NULL (which closes the camera or microphone) and takes the
   source element out, so that a consumer attaching the stream later cannot
   start the device again; it then produces nothing. A failure of any of these
   steps is logged (`log::warn!`): a device left open must not go unnoticed.
2. **`media_capture.rs`:** a fixed number for a fractional constraint was
   written to a caps field literally named `"name"` instead of the field
   (`frameRate`, `aspectRatio`). Upstream line 68.
3. **`media_stream.rs`:** `vp8enc`'s `error-resilient` (flags) and `rtpvp8pay`'s
   `picture-id-mode` (enum) are set from their nicks (`property_from_str`)
   instead of from a string value (upstream lines 144 and 150). This is a
   Servo bug, not a GStreamer one: both are typed properties, and gstreamer-rs's
   typed `property` panics when given a string for them. It panicked the script
   thread (a crashed tab) as soon as a capture stream was shown in a `<video>`.
   Found by the fake-device runs.

## Backports from post-0.8 Servo (lane med-1, EA0)

Three small fixes from merged upstream pull requests, applied unchanged apart from a `Mercury:` marker
comment at each (they apply cleanly to 0.8.0 and fall away at the next Servo bump):

4. **`player.rs`, servo/servo#49784:** the `GstPlay` signal handlers and the `source-setup` closure hold
   weak references to the player's inner state and to the `ServoSrc`, a new `seek_lock` field lets teardown
   release a `seek_data` that is blocked waiting for the script thread, and `Drop for GStreamerPlayer`
   disposes the `GstPlay` from the dropping thread (so its thread is joined there) and checks that nothing
   else holds it. Fixes crashes and hangs when a media element is torn down while GStreamer is still posting
   messages (it matters more once a second source type, MSE, exists).
5. **`player.rs`, servo/servo#49889:** progressive download is attempted only when the user cache directory
   can be created (`ensure_download_directory`; Windows is no longer excluded, upstream issue #43421 for
   Android), and a duration that arrives together with the metadata is reported as `DurationChanged` (the
   two GstPlay signals have no guaranteed order).
6. **`render.rs`, servo/servo#49661:** a frame whose rows are padded (VideoToolbox aligns them) is copied
   row by row into a packed buffer, so the consumer of `VideoFrame` sees the picture it expects. Reads the
   stride from the mapped frame, not from the caps.

## Media Source Extensions glue (lane med-1, MSE by Plan B)

MSE is Mercury's own code, not GStreamer's `mse` plugin (the spike said no, R17): `crates/mercury-mse`
holds the parsers, the coded-frame processing, the track buffers and the MediaSource model, and
`crates/mercury-mse-gst` holds `MseSrc`, a source element (a `GstBin` with one `appsrc` per track) registered
as the URI handler of `mercury-mse://`. This crate only connects them to Servo's player. `Cargo.toml` gains
path dependencies on both crates. Everything new is in `mercury_mse.rs`; the hooks in upstream files are:

7. **`mercury_mse.rs` (new file):** `register()` (the element and its URI handler; a failure is logged and
   leaves MSE unsupported), `create_media_source(events)` (an `MseBackend` whose `EventSink` sends on the
   `GenericCallback`, with `GstTypeSupport` and the default quota `Limits`), `is_type_supported(mime)`,
   `uri_for(player id)` and `attach(source element, backend)`.
8. **`lib.rs`:** `mod mercury_mse`, `register()` after the plugins are loaded, and the two `Backend`
   methods `create_media_source` and `is_type_supported_mse`.
9. **`player.rs`:** `StreamType::MediaSource` builds the same `playbin3` with `uri=mercury-mse://<id>` and
   no download buffering; the `source-setup` closure attaches the feed (`mercury_mse::attach`) and releases
   the caller of `setup` (there is no `need-data` to wait for before the page appends anything);
   `Player::connect_media_source` stores the backend and runs `setup`, so it must come first; `seek` and
   `set_playback_rate` accept the new stream type (the duration check in `seek` is skipped for it: the
   MediaSource owns the duration); `push_data` is never used. Hunks are marked `Mercury: (MED-1)`.

Tested by `mercury-mse` and `mercury-mse-gst`'s own suites (light pool, no Servo). The Servo-side glue
compiles (`cargo check -p servo-media-gstreamer`) and has **not been run in a window** (the screen was locked);
`tests/pages/mse-*.html` and `docs/developer-guide/media-source.md` say how to run it. Dropped when a Servo
release has its own MSE (servo/servo#44403) and Mercury moves to it.
