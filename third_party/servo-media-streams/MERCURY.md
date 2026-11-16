# Patched servo-media-streams 0.8.0

This directory vendors `servo-media-streams` 0.8.0 under its original MPL-2.0
license (declared in `Cargo.toml`; MPL-2.0 headers; no licence file in the
crate). It is the crate exactly as published on crates.io (checksum
`6a36e094d79e768473b699e84513547521476529f5b2253dad9edf3a337dd116`), built from
https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/media/streams`,
added unchanged in commit `ea292e9`.

It is part of the media-capture patch described in
`third_party/servo-script/MERCURY.md`.

## Changes

1. **`lib.rs`:** `MediaStream::stop(&mut self)`, a trait method with an empty
   default, which `servo-script` calls to release a camera or microphone. The
   stream stays registered: several consumers `expect()` it in the registry.
   Upstream nothing ever stops a device stream (the registry's only
   `unregister_stream` caller is the stream's own `Drop`, which cannot run
   while the registry holds it).
