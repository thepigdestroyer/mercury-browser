# GstMSE spike fixtures

Short synthetic fragmented MP4 and WebM files that every scenario of the GstMSE spike
(the GstMSE spike of lane `sp-mse`, now the acceptance fixtures of lane `med-1`) appends through MSE. They are original content made with GStreamer's
own encoders by [`gen.sh`](gen.sh); nothing is downloaded and nothing is taken from another project. The
committed bytes (listed with their SHA-256 in [`SHA256SUMS`](SHA256SUMS)) are the reference: the MP4
muxers write the current time into the movie header, so rerunning `gen.sh` gives equivalent files, not
identical ones. Committed bytes make the Intel and the Apple Silicon runs comparable.

Total size: 1,017,025 bytes (budget 1.5 MB). Read them from Rust through `mercury_mse_testkit::load(name)`, which
splits a fixture into its initialisation segment and media segments and derives the type string.

## Content

Every file is **8.0 s at 25 fps with a key frame every 2 s (50 frames)**, in **2 s fragments** (four media
segments) unless the table says otherwise.

* Video: a solid **red** background for the first 4 s (frames 0 to 99) and a solid **blue** one for the last
  4 s (frames 100 to 199), with a small black ball moving over it (`videotestsrc pattern=ball`). The mean
  colour of a decoded frame therefore says which half of the clip it came from, after any seek, quality
  switch or codec change. Frame sizes 320x180 ("low") and 640x360 ("high").
* Audio: a 440 Hz sine, mono, 48 kHz, 8 s.
* One track per file (as DASH delivers them) except `h264-aac`.
* The two renditions of a codec (`h264-low`/`h264-high`, `vp9-low`/`vp9-high`) have **identical key frame
  and fragment boundaries** (start 0, 2000, 4000, 6000 ms, 2000 ms each), so a quality switch can happen
  at any segment boundary. `h264-bframes` has them too.
* Fragmented the way MSE wants: MP4 has `ftyp` + `moov` (with `mvex`), then one `styp` + `moof` + `mdat` per
  fragment, no `sidx`, `tfdt` version 0; WebM has an EBML header and a Segment of unknown size (streamable
  muxing), then one Cluster per fragment, no Cues.

| Name | File | Bytes | Media segments | Type string (derived from the file) | SHA-256 |
|---|---|---:|---|---|---|
| `h264-low` | `fmp4/video-h264-low.mp4` | 59,125 | 4 x 2000 ms | `video/mp4; codecs="avc1.42C00C"` | `bb800cf6e9a7df2cf29432dfafa4a237359c3f5e2f130a892d7b07b95a7ede6b` |
| `h264-high` | `fmp4/video-h264-high.mp4` | 63,201 | 4 x 2000 ms | `video/mp4; codecs="avc1.42C01E"` | `fcacd5eda0b608168a7cc941ca43d4302876897d6fcf89273a8fb53cf59cb841` |
| `aac` | `fmp4/audio-aac.mp4` | 34,668 | 2005, 2006, 2005, 1984 ms | `audio/mp4; codecs="mp4a.40.2"` | `55d4d575231bbb67b5b9d9a70df55ae861e02ceea584f010434baeea1da6ddbe` |
| `h264-aac` | `fmp4/muxed-h264-aac.mp4` | 94,425 | 4 (start 0, 1984, 3989, 5995 ms) | `video/mp4; codecs="avc1.42C00C, mp4a.40.2"` | `edf990c63f13fcc5b38f088b005d7150e0f6920bb2e17276e17bff0594ffc10b` |
| `h264-bframes` | `fmp4/video-h264-bframes.mp4` | 68,130 | 4 x 2000 ms | `video/mp4; codecs="avc1.64001E"` | `cbb2a6e70fb7b9d422abfb28aa22b4401baf91f5bf4e53339da7c8b3e672c571` |
| `vp9-low` | `webm/video-vp9-low.webm` | 73,603 | 4 x 2000 ms | `video/webm; codecs="vp09.00.11.08"` | `4effce4722f6d5ccfaa75b3342a050576ba14b829c138d3a8d1b5d32b28ccdfb` |
| `vp9-high` | `webm/video-vp9-high.webm` | 112,844 | 4 x 2000 ms | `video/webm; codecs="vp09.00.21.08"` | `7b75f6c4df54416d29def986565722dc377f600a006a31f38c90c53e13382022` |
| `opus` | `webm/audio-opus.webm` | 27,676 | **16 x 500 ms** | `audio/webm; codecs="opus"` | `8c3ed7c0eccbef7d012ed1532402d18678e1bf80cf4035c207c5f4973aa56e88` |
| `av1-low` | `webm/video-av1-low.webm` | 52,986 | 4 x 2000 ms | `video/webm; codecs="av01.0.00M.08"` | `0913c716e09c0390253992e2c50a229ccd7c78190e2b6877cc14e9320b7fb53a` |
| `av1-low-av1c` | `webm/video-av1-low-av1c.webm` | 52,957 | 4 x 2000 ms | `video/webm; codecs="av01.0.00M.08"` | `d7d133abc40d49e190d68c71e3535d4c59e64022354f3ecc1cd6c418de7cbbc7` |

Priorities in the lane brief: P0 `h264-low`, `h264-high`, `aac`; P1 `h264-aac`, `vp9-low`, `vp9-high`, `opus`,
`h264-bframes`; P2 `av1-low`. All nine were made; the tenth, `av1-low-av1c`, is a control added after a review
(below).

## Things that differ from "four 2 s segments", and why

* **`aac`: 2005, 2006, 2005, 1984 ms.** An AAC frame is 1024 samples (21.33 ms); 94 of them make 2.005 s,
  which no 2 s fragment duration can hit. `cmafmux` closes a fragment before it would exceed
  `fragment-duration`, so with 2.000 s it made 93 frames per fragment and a fifth, tiny fragment (375 frames
  in all). `gen.sh` sets the audio fragment duration to 2.006 s: 94 + 94 + 94 + 93 frames, four fragments, 8.000 s.
  Audio and video fragment boundaries are therefore close but not identical, as in real DASH content.
* **`h264-aac`: start times 0, 1984, 3989, 5995 ms.** One `moof` carries both tracks (two `traf`), cut at the
  video key frames; the segment start is the earliest `tfdt` of the two tracks (the audio one), and the
  duration reaches to the latest end. The muxer is `isofmp4mux` because `cmafmux` takes one stream.
* **`opus`: 16 segments of 500 ms.** `webmmux` starts a new Cluster every 500 ms in a file without video and
  has no cluster-duration property. The 8.000 s are 400 Opus frames of 20 ms (the encoder adds one frame of
  look-ahead, so the input is 399 frames); decoded, the file is 7.98 s long after the decoder removes the
  Opus pre-skip.
* **`vp9-*`: `vp09.00.11.08` and `vp09.00.21.08`.** VP9 streams carry no level. The type string takes
  profile and bit depth from the first key frame's header and derives the level from the frame size and
  rate (VP9 level table: 320x180 at 25 fps is level 1.1, 640x360 is 2.1). The libvpx output has no
  CodecPrivate. When no key frame can be read the splitter falls back to the short form `vp9`.
* **`h264-bframes`: High profile (`avc1.64001E`), frame reordering on.** VideoToolbox's `vtenc_h264` has no
  profile property, the profile comes from the caps after the encoder (`video/x-h264,profile=high`). The
  `trun` boxes carry composition time offsets (flag 0x800, non-zero values: a unit test checks), so
  presentation order differs from decode order. AAC priming (an edit list for the encoder delay) was
  not attempted: `voaacenc` adds none, in this file or in `aac`. `vtenc_h264` is an Apple
  encoder: on another Mac or OS the file would come out differently, which is one more reason the bytes are
  committed.
* **`av1-low`: codec_data added by hand.** `svtav1enc` writes no `codec_data`, so `webmmux` would write no
  CodecPrivate, which an AV1 track in WebM needs (the `av1C` record). `gen.sh` adds the 4-byte record
  `81 00 0c 00` (version 1; profile 0 and level index 0; tier 0, 8-bit, 4:2:0) that `av1parse` reports for
  the stream, in a caps filter. `rav1enc` writes it by itself but has no key frame interval setting, so
  `svtav1enc` is used. `svtav1enc` also gave duplicate and jumping Cluster timestamps after the colour
  switch (`concat` changes the segment there) until `identity single-segment=true` was put in front of it.
  The type string is `av01.0.00M.08`: level index 0 is level 2.0, as the encoder chose; `rav1enc` would say
  level index 31.
* **`av1-low-av1c`: the control for `av1-low`.** The same stream settings with the **complete** `av1C` record as
  CodecPrivate: the four bytes `81 00 0c 00` followed by the sequence header OBU (`0a 0e` and 14 payload bytes,
  16 bytes in all) that `svtav1enc` writes at the start of the stream. Apple's VideoToolbox only starts an AV1
  decode session when the record carries the sequence header (a community recipe, not an Apple statement), so
  a failure of `av1-low` on a Mac could be the fixture's fault and not the chip's; this file takes that
  explanation away. `gen.sh` reads the sequence header from a first, raw encode (it depends on the settings,
  not on the pictures) and checks that it occurs at least twice in the muxed file (CodecPrivate and the first
  key frame). It is a fresh encode, so its frames are not byte for byte those of `av1-low` (the encoder is
  not bit-reproducible: 52,957 against 52,986 bytes); the four fixed bytes and the type string are the same.
  `gen.sh <dir> av1c` makes only this file. Tests: `mercury_mse_testkit::av1` (the two records, the OBU walk, the
  sequence header found again in the first media segment).

## The fixtures of decision D7 (lane med-1b: HEVC, AC-3, E-AC-3, FLAC, MP3, Vorbis)

Seven more files, made by `gen.sh <dir> d7` (the other ten keep their committed bytes). Each is **8 s and at
most 100 KB** (the brief's limit; the seven together are about 350 KB). The video is the same red/blue ball
(320x180, 25 fps, a key frame every 50 frames); the audio is the same 440 Hz mono sine. Items that need a
playing stream use them through `mercury_mse_testkit::load(name)`; the parser tests check each one whole and
under every chunk plan (`crates/mercury-mse/tests/parse_codecs.rs`).

| Name | File | Bytes | Media segments | Type string (derived from the file) | Notes |
|---|---|---:|---|---|---|
| `hevc-low` | `fmp4/video-hevc-low.mp4` | 32,273 | 4 x 2000 ms | `video/mp4; codecs="hvc1.1.6.L60.B0"` | `vtenc_h265`, hvc1, no B-frames |
| `ac3` | `fmp4/audio-ac3.mp4` | 65,116 | 63, 63, 63, 61 frames of 32 ms | `audio/mp4; codecs="ac-3"` | `avenc_ac3 bitrate=64000`, 48 kHz mono |
| `eac3` | `fmp4/audio-eac3.mp4` | 65,118 | the same | `audio/mp4; codecs="ec-3"` | `avenc_eac3 bitrate=64000`, 48 kHz mono |
| `flac` | `fmp4/audio-flac.mp4` | 81,018 | 4 | `audio/mp4; codecs="flac"` | **16 kHz** mono (see below) |
| `mp3` | `fmp4/audio-mp3.mp4` | 38,572 | 84, 84, 84, 83 frames of 24 ms | `audio/mp4; codecs="mp4a.6B"` | `lamemp3enc bitrate=48`, 334 frames of 1152 samples in, 335 out (8.040 s) |
| `vorbis` | `webm/audio-vorbis.webm` | 14,435 | 16 (the last 299 ms) | `audio/webm; codecs="vorbis"` | `vorbisenc bitrate=32000` |
| `flac-webm` | `webm/audio-flac.webm` | 80,878 | 14 x 576 ms | `audio/webm; codecs="flac"` | `matroskamux`: DocType `matroska` |

* **Why FLAC is 16 kHz.** A full-band 440 Hz sine at 48 kHz does not compress below 170 KB in 8 s (FLAC is
  lossless and the sine's rounding noise costs about 3.7 bits per sample); at 16 kHz the file is 81 KB. The
  rate is part of what the parser reads (`TrackConfig::sample_rate` comes from STREAMINFO), so the fixture
  also checks that a rate other than 48 kHz survives.
* **Segments.** The Rust muxers close a fragment before it would exceed `fragment-duration`, so the audio
  fragment duration is set just above a whole number of frames (2.017 s for 63 AC-3 frames), as for `aac`.
* **`mp3` comes from `mp4mux`** (gst-plugins-good): the Rust MP4 muxers do not take MPEG audio. It writes no
  `styp`, and ends the file with an `mfra` index that no live stream carries; `gen.sh` cuts it (the last four
  bytes of its `mfro` box give its size), so that init segment plus media segments are the whole file.
* **`flac-webm` is Matroska.** `webmmux` takes only Vorbis and Opus audio; FLAC goes through `matroskamux`, so
  the EBML DocType is `matroska` (the parser accepts both). The type string is still `audio/webm`.
* **Codecs of the new files:** `hvcC` with VPS, SPS and PPS arrays; `dac3` (3 bytes); `dec3` (5 bytes); `dfLa`
  with STREAMINFO; `mp4a` with `esds` object type 0x6B; WebM `A_VORBIS` (Xiph-laced `CodecPrivate`) and
  `A_FLAC` (`fLaC` marker and blocks).
* Encoders: `vtenc_h265` (VideoToolbox, part of macOS), `avenc_ac3` and `avenc_eac3` (libav, LGPL,
  dynamically linked in the bundle), `flacenc`, `vorbisenc` (Xiph, BSD-3-Clause), `lamemp3enc` (LAME, LGPL).
  **`x265enc` is GPL and is not used.** Like the others, they run at generation time only.

## Encoders and settings

All from the bundled GStreamer 1.28.9 (`scripts/macos/gstreamer.sh`), in a private plugin registry; plugin
versions: `openh264enc`, `voaacenc`, `vp9enc`, `opusenc`, `webmmux`, `svtav1enc`, `vtenc_h264` 1.28.9;
`cmafmux`, `isofmp4mux` 0.15.4 (gst-plugins-rs). **Never `x264enc` or `x265enc`: they are in the framework
but GPL.** The encoder libraries are run at generation time and are not linked into anything; none is GPL
(openh264 BSD-2-Clause, libvpx and libopus BSD-3-Clause, SVT-AV1 BSD-3-Clause-Clear with the AOM patent
licence, vo-aacenc Apache-2.0, VideoToolbox is part of macOS).

| Fixture | Encoder and settings | Muxer |
|---|---|---|
| `h264-low` | `openh264enc bitrate=150000 gop-size=50 rate-control=bitrate complexity=low enable-frame-skip=false multi-thread=1`, `h264parse`, avc/au | `cmafmux fragment-duration=2000000000` |
| `h264-high` | same, `bitrate=400000`, 640x360 | same |
| `aac` | `voaacenc bitrate=32000`, `aacparse`, raw | `cmafmux fragment-duration=2006000000` |
| `h264-aac` | `h264-low`'s video encoder and `aac`'s audio encoder | `isofmp4mux fragment-duration=2000000000` |
| `h264-bframes` | `vtenc_h264 realtime=false allow-frame-reordering=true max-keyframe-interval=50 bitrate=400` (kbit/s), caps `profile=high` | `cmafmux fragment-duration=2000000000` |
| `vp9-low` | `vp9enc target-bitrate=150000 keyframe-max-dist=50 deadline=1 cpu-used=4 end-usage=vbr threads=1 lag-in-frames=0 auto-alt-ref=false` | `webmmux streamable=true` |
| `vp9-high` | same, `target-bitrate=400000`, 640x360 | same |
| `opus` | `opusenc bitrate=24000` (frames of 960 samples) | `webmmux streamable=true` |
| `av1-low` | `svtav1enc target-bitrate=150 intra-period-length=49 preset=10`, `av1parse`, obu-stream/tu, codec_data `81000c00` | `webmmux streamable=true` |
| `av1-low-av1c` | same, with codec_data `81000c00` followed by the sequence header OBU | `webmmux streamable=true` |

## Commands

`gen.sh` runs these (it sets the private registry, so that nothing of the system or the Mercury profile is
read, and links only the plugins it needs into a scratch directory: a scan of the whole framework loads
`libgstvulkan`, which can hang inside MoltenVK on an Intel Mac with an AMD GPU). With the colours
`RED=4294901760`, `BLUE=4278190335`, `BLACK=4278190080` (big-endian ARGB), and `SOURCES(w,h)` for the two
video sources that feed the `concat` element `c`:

```
SOURCES(w,h) = videotestsrc pattern=ball num-buffers=100 background-color=$RED  foreground-color=$BLACK
                 ! video/x-raw,width=w,height=h,framerate=25/1 ! queue ! c.
               videotestsrc pattern=ball num-buffers=100 background-color=$BLUE foreground-color=$BLACK
                 ! video/x-raw,width=w,height=h,framerate=25/1 ! queue ! c.
AUDIO        = audiotestsrc wave=sine freq=440 num-buffers=375 samplesperbuffer=1024
                 ! audio/x-raw,rate=48000,channels=1

# h264-low (h264-high: 640x360 and bitrate=400000)
gst-launch-1.0 -q concat name=c ! videoconvert ! video/x-raw,format=I420 \
  ! openh264enc bitrate=150000 gop-size=50 rate-control=bitrate complexity=low enable-frame-skip=false multi-thread=1 \
  ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au \
  ! cmafmux fragment-duration=2000000000 ! filesink location=video-h264-low.mp4  SOURCES(320,180)

# aac
gst-launch-1.0 -q AUDIO ! voaacenc bitrate=32000 ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw \
  ! cmafmux fragment-duration=2006000000 ! filesink location=audio-aac.mp4

# h264-aac
gst-launch-1.0 -q isofmp4mux name=m fragment-duration=2000000000 ! filesink location=muxed-h264-aac.mp4 \
  concat name=c ! videoconvert ! video/x-raw,format=I420 \
  ! openh264enc bitrate=150000 gop-size=50 rate-control=bitrate complexity=low enable-frame-skip=false multi-thread=1 \
  ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au ! m.  SOURCES(320,180) \
  AUDIO ! voaacenc bitrate=32000 ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw ! m.

# h264-bframes
gst-launch-1.0 -q concat name=c ! videoconvert ! video/x-raw,format=I420 \
  ! vtenc_h264 realtime=false allow-frame-reordering=true max-keyframe-interval=50 bitrate=400 \
  ! video/x-h264,profile=high ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au \
  ! cmafmux fragment-duration=2000000000 ! filesink location=video-h264-bframes.mp4  SOURCES(640,360)

# vp9-low (vp9-high: 640x360 and target-bitrate=400000)
gst-launch-1.0 -q concat name=c ! videoconvert ! video/x-raw,format=I420 \
  ! vp9enc target-bitrate=150000 keyframe-max-dist=50 deadline=1 cpu-used=4 end-usage=vbr threads=1 lag-in-frames=0 auto-alt-ref=false \
  ! webmmux streamable=true ! filesink location=video-vp9-low.webm  SOURCES(320,180)

# opus (399 input frames of 960 samples: 400 Opus frames with the encoder's look-ahead)
gst-launch-1.0 -q audiotestsrc wave=sine freq=440 num-buffers=399 samplesperbuffer=960 \
  ! audio/x-raw,rate=48000,channels=1 ! opusenc bitrate=24000 \
  ! webmmux streamable=true ! filesink location=audio-opus.webm

# av1-low
gst-launch-1.0 -q concat name=c ! identity single-segment=true ! videoconvert ! video/x-raw,format=I420 \
  ! svtav1enc target-bitrate=150 intra-period-length=49 preset=10 \
  ! av1parse ! "video/x-av1,stream-format=(string)obu-stream,alignment=(string)tu,codec_data=(buffer)81000c00" \
  ! webmmux streamable=true ! filesink location=video-av1-low.webm  SOURCES(320,180)
```

## How they were checked

* `cargo test --release -p mercury-mse-testkit` (seconds, no GStreamer): the SHA-256 of every file against
  `SHA256SUMS`, the total size, segment counts, start and duration of each segment, the contents of each
  initialisation segment, the derived type string against `FIXTURES`, B-frame composition offsets, chunking,
  time shifting, tiling, `pssh`, truncated and damaged input.
* Decoded with `qtdemux`/`matroskademux` and `avdec_h264`, `vp9dec`, `dav1ddec`, `avdec_aac`, `opusdec`
  (GStreamer 1.28.9, Intel Mac): every video file gives 200 frames, the first 100 red and the last 100
  blue (a 1x1 pixel crop away from the ball); `aac` decodes to 8.000 s of audio and `opus` to 7.98 s.
* `cargo run --release -p mercury-mse-testkit --example dump -- FILE...` prints how a file splits.

## Licence

Original synthetic content, PolyForm Noncommercial 1.0.0 (see `REUSE.toml`: media files cannot carry a
header). `gen.sh` has its own SPDX header.
