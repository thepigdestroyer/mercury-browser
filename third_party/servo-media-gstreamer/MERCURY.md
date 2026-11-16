# Patched servo-media-gstreamer 0.8.0

This directory vendors `servo-media-gstreamer` 0.8.0, Servo's GStreamer media
backend, under its original MPL-2.0 license (declared in `Cargo.toml`; MPL-2.0
headers; no licence file in the crate). It is the crate exactly as published on
crates.io (checksum
`7bb5bc60a53436a224a57f23059a494bf84296119f1c6e8f5bbba8a2af3fa96f`), built from
https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path
`components/media/backends/gstreamer`, added unchanged in commit `ea292e9`.

It is part of the media-capture patch described in
`third_party/servo-script/MERCURY.md`.

## Changes

1. **`media_stream.rs`:** `GStreamerMediaStream::stop()` sets the stream's
   pipeline to NULL (which closes the camera or microphone) and takes the
   source element out, so that a consumer attaching the stream later cannot
   start the device again; it then produces nothing.
2. **`media_capture.rs`:** a fixed number for a fractional constraint was
   written to a caps field literally named `"name"` instead of the field
   (`frameRate`, `aspectRatio`). Upstream line 68.
