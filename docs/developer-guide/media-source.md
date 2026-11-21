---
title: Media Source Extensions
description: How Mercury implements MediaSource and SourceBuffer on its own backend, how the pieces fit, how to turn the feature on for development, how to run the test pages, and what is still missing.
---

# Media Source Extensions

Media Source Extensions (MSE) let a page build a media stream from segments it fetches itself: `new MediaSource()`, `URL.createObjectURL(mediaSource)` or `video.srcObject = mediaSource`, `addSourceBuffer(type)`, `appendBuffer(bytes)`. DASH players (dash.js, Shaka) and hls.js depend on it, and so do YouTube and most large video sites.

**State: written, compiled, tested in pieces, not yet run in a browser window, and off for everyone.** The preference `dom_media_source_enabled` stays pinned off in `crates/talaria-servo/src/engine_prefs.rs`. The first action of the next round is to run the pages below in a real window and then decide when to flip the pin.

## How it is built

Servo 0.8 has no MSE, and GStreamer's own `mse` plugin did not pass the spike (`research/R17-mse-spike.md`: seek latency, split audio and video, teardown). So the whole of MSE below the JavaScript objects is Mercury's own Rust code, behind one trait:

```
 JS (MediaSource, SourceBuffer, SourceBufferList)    third_party/servo-script/dom/mse/        thin DOM
        |  Arc<dyn MediaSourceBackend> calls; MseEvent back through a GenericCallback
 servo-media: Backend::create_media_source          third_party/servo-media, servo-media-player
        |
 mercury_mse::MseBackend                             crates/mercury-mse                       Servo-free, GStreamer-free
        |    byte-stream parsers (fMP4, WebM) -> coded-frame processing -> track buffers
        |    the MediaSource and SourceBuffer state machines, one worker thread per MediaSource
        |  FrameFeed (pull)
 mercury_mse_gst::MseSrc  (uri mercury-mse://<n>)    crates/mercury-mse-gst                   one appsrc per track
        |
 playbin3                                            third_party/servo-media-gstreamer/player.rs
```

- **`crates/mercury-mse`** owns every spec algorithm that needs no DOM object: the readyState, the duration rules and truncation, `endOfStream`, the buffered intersection, `abort`, `remove`, `changeType`, the append window, the timestamp offset, the mode, coded-frame processing, quota and eviction (150 MiB video, 12 MiB audio, 1 MiB text per source buffer; the data behind the playback position minus 5 s goes first), and the element's readiness from the buffered ranges. Its contract is `src/api.rs` (`MediaSourceBackend`, `MseEvent`, `FrameFeed`, the types).
- **`crates/mercury-mse-gst`** is the GStreamer half: the `MseSrc` element and the decoder-registry type support. It is registered as the URI handler of `mercury-mse://`, so `playbin3` creates it, and the player's `source-setup` hands it the feed.
- **The DOM** (`third_party/servo-script/dom/mse/`) is carried in part from Servo pull request #47269 by decomoraes (MPL-2.0) and thinned: it keeps what is JavaScript-visible (the objects, the events and their tasks, the `updating` flag as the specification's tasks publish it, the object URL store) and forwards everything else to the backend. The backend's events (`InitSegment`, `AppendDone`, `AppendError`, `RemoveDone`, `DurationChanged`, `SourceEnded`, `BufferedChanged`) come back as media-element tasks.
- **The element** (`htmlmediaelement.rs`) attaches a MediaSource that a `blob:` URL or `srcObject` names, creates the player with `StreamType::MediaSource` and connects it to the backend, takes the duration, `buffered`, `seekable` and the readiness from the backend, ignores the media engine's own duration and metadata, and never pushes bytes to the player.
- **The vendored crates** change as `third_party/servo-script/MERCURY.md`, `servo-script-bindings/MERCURY.md`, `servo-media/MERCURY.md`, `servo-media-player/MERCURY.md` and `servo-media-gstreamer/MERCURY.md` list, with the hunks in the hot file `htmlmediaelement.rs` marked `Mercury (MED-1)`. A vendored crate depends on the two workspace crates by path (`make servo-patches-check`, `cargo fmt --all` and clippy accept it: the reverse direction is the forbidden one).
- **dav1d** (the AV1 software decoder) joined the bundle as one of Mercury's own plugins (`scripts/macos/collect-gstreamer.sh`, `MERCURY_PLUGINS`); no decoder rank is changed anywhere.

Supported byte streams this round: fragmented MP4 and WebM; H.264, VP9, VP8, AV1, AAC and Opus; one track per kind in a source buffer. HEVC, AC-3, E-AC-3, MP3, FLAC, Vorbis, MPEG-TS and raw `audio/mpeg` streams are refused by `isTypeSupported` and `addSourceBuffer`.

## Turning it on for development

The preference is off. To try the DOM, start the browser with the environment variable `MERCURY_DEV_MSE=1`:

```bash
MERCURY_DEV_MSE=1 ./target/release/mercury-browser http://127.0.0.1:<port>/pages/mse-basic.html
```

`engine_prefs::apply` reads it once at start-up (only the value `1` counts) and sets `dom_media_source_enabled` after the pins. Without it the interfaces do not exist (`typeof MediaSource` is `"undefined"`), which the pages report as their first failure. The tests `media_source_is_off_by_default_and_pinned` and `the_development_switch_turns_media_source_on_and_nothing_else` in `engine_prefs.rs` hold both sides.

## The test pages

`tests/pages/mse-{basic,split-av,switch,seek,remove,quota,object-url}.html` exercise the DOM against the fixtures of `crates/mercury-mse-testkit/fixtures/` (8 s at 25 fps, red for the first 4 s and blue for the last 4 s, a key frame every 2 s). Each prints one `PASS` or `FAIL` line per check and writes the totals into `document.title`, which ends with `(done)` when the page has finished.

**They have not been run** (the screen was locked while they were written). When the screen is free:

1. Serve `tests/` and the fixtures: `scripts/serve-tests.sh` prints the port. The pages read the fixtures from `/mse-fixtures/<path>` (for example `/mse-fixtures/fmp4/video-h264-low.mp4`), a route of `crates/mercury-devtools`' `serve-tests` that maps to `crates/mercury-mse-testkit/fixtures/`; add `?fixtures=<base url>` to read them from elsewhere.
2. Run each page with `MERCURY_DEV_MSE=1`, in a window, once per page; `--screenshot out.png --screenshot-delay 20000` also works, and the title is the verdict.
3. A page that stops early prints `FAIL the page stopped: ...` with the stack. `mse-quota.html` appends about 150 MiB and takes the longest; `mse-seek.html` prints the seek latencies as `INFO` lines (the targets are the first frame within 500 ms and a resume within 1000 ms after the late append).

What the pages assume from the engine: `canvas.drawImage(video)` reads the current frame (a page prints `INFO` and skips the colour check if it cannot), `video.play()` is allowed on a muted video, and `fetch` works against the test server.

## Tests that run without a window

- `cargo test --release -p mercury-mse -p mercury-mse-testkit -p mercury-mse-gst` (light pool): the parsers, the processor, the model and the GStreamer feed with `appsink`s, on the same fixtures; scenarios C01 to C15 and T01 to T12 of the spike, ported.
- `cargo test --release -p talaria-servo engine_prefs`: the preference and its development switch.
- `make servo-patches-check`, `make vendored-tests`: the vendored crates.

The DOM is thin on purpose: what could be tested without a page is in the crate. What is left in the DOM and cannot be tested without a window is listed in the lane report (`.superpowers/sdd/lane-med-1-report.md`): the event plumbing, the `updating` flag's timing, and the media element's readiness and seek hooks.

## What is still missing

1. Run the pages and the DOM in a window; run WPT `media-source/` with the preference on (start from the expectations of Servo pull request #47269); run dash.js, Shaka and hls.js on local fixtures; then YouTube and Twitch by hand; then flip the pin.
2. `canPlayType` and `mediaCapabilities` for MSE types, `powerEfficient`, HEVC, AC-3, E-AC-3, MP3, FLAC, Vorbis, MPEG-TS and raw audio streams, with fixtures for each; AV1 on a real M-series Mac.
3. `getVideoPlaybackQuality()` (dash.js and Shaka read it), `ManagedMediaSource`, `BufferedChangeEvent`, `MediaSourceHandle`, MSE in workers, `SourceBuffer.textTracks` and the contents of `audioTracks` and `videoTracks`.
4. Stall detection at the end of the buffer (`waiting` is driven by the position updates every 500 ms and the player's state changes, not by an underrun signal), a stale `AppendDone` that arrives after `abort()` and a new append in the same task, A/V sync and quality switches on real DASH content, long-run memory and the quota numbers tuned on real players.
5. Encrypted Media (the `pssh` boxes are already extracted raw), forced mute and autoplay (EA3, EA4), the Linux build of the new crates.
6. Drop Mercury's DOM carry when a Servo release has MSE (servo/servo#44403).
