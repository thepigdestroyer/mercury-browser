---
title: Media Playback
description: Which audio and video formats Mercury Browser plays on macOS, what it writes to disk while playing, and why some video sites don't work.
---

# Media playback

Mercury plays audio and video through Servo's GStreamer backend. On macOS the GStreamer libraries (version 1.28.9) are inside the app, so nothing needs to be installed.

## What plays

Checked on Apple Silicon with the bundled GStreamer 1.28.9:

| Format | Status |
|---|---|
| MP4 with H.264 video and AAC audio | Plays. Video is decoded in hardware (VideoToolbox). |
| WebM with VP9 video and Opus audio | Plays. VP9 is decoded in hardware (VideoToolbox) where the Mac supports it. |
| MP3, WAV | Play. |
| Ogg (Opus, Vorbis, Theora) | **Not supported for now.** |
| Web Audio | Works, including `decodeAudioData` for MP3 and WAV. |

Ogg is left out of the macOS build for now because GStreamer 1.24.13 and later (the change arrived
in a 1.24 patch release) can end Ogg playback about a second after it starts: a defect in
GStreamer's Ogg demuxer, not in the files. Pages are
told Ogg is not supported, so sites that offer another format use it instead.

Sites that ask `canPlayType('video/webm; codecs="vp9, opus"')` are told "no": Servo's list of
GStreamer plugins does not include the Opus parser that this check looks for. WebM files with Opus
audio still play when a page uses them directly.

The first start after installing or updating Mercury can take about ten seconds before media
is ready, while macOS checks the bundled libraries. Later starts are fast.

## Video sites

Servo 0.8 has no Media Source Extensions (MSE). Most large video sites, YouTube among them, stream through MSE in adaptive chunks, so their videos may not play in Mercury. A page that links or embeds a plain MP4 or WebM file plays it.

When a site's video doesn't play, open a direct MP4 or WebM file to tell a site problem from a media problem: if the file plays, the site depends on something Servo doesn't support yet.

Also not available: DRM-protected video (Encrypted Media Extensions), picture-in-picture, and camera and microphone (WebRTC and `getUserMedia` are off; camera and microphone support is being merged). The tab audio indicator, muting a tab and macOS Now Playing are being merged too.

## How the bundled GStreamer is kept separate

Mercury loads GStreamer plugins only from its own `lib` folder
(`Mercury Browser.app/Contents/MacOS/lib`), keeps its plugin registry in its own cache folder
(`~/Library/Caches/io.github.thepigdestroyer.mercury-browser/gstreamer`), and ignores GStreamer
plugins, registries and GIO modules that other software installed for your user account
(`GST_PLUGIN_PATH`, `~/.local/share/gstreamer-1.0/plugins`, `~/.cache/gstreamer-1.0`,
`GIO_EXTRA_MODULES`).

**What media playback writes to disk, and when it is removed.** Everything is under Mercury's
cache folder, `~/Library/Caches/io.github.thepigdestroyer.mercury-browser/`, and nothing is
written to `~/.cache`, `~/.local` or `~/.config`:

| Where | What | Removed |
|---|---|---|
| `gstreamer/registry-<version>-<architecture>.bin` | GStreamer's list of the bundled plugins (no media). | Kept; rebuilt when Mercury's GStreamer changes. |
| `media-buffers/<process id>-<time>/` | One private folder (readable only by you) for each running Mercury. While media plays, GStreamer keeps a copy of each stream it downloads in its `cache` subfolder, so that playback can seek without downloading again. These copies are of what you played. | When Mercury quits normally, the whole folder. If Mercury was killed or crashed, at its next start (a folder still locked by a running Mercury is left alone, so several copies of Mercury can run at once). |

If the media folder cannot be created, media still plays, without those download copies.

## Linux

A Linux build uses the GStreamer installed on the system. `make install-linux-deps` installs the plugin sets on Debian and Ubuntu; see [Install Mercury](installation.md#linux). `gst-inspect-1.0 avdec_h264` and `gst-inspect-1.0 vp9dec` show whether a decoder is there.

## Diagnose playback

Start Mercury from Terminal with media logging:

```bash
RUST_LOG="talaria_servo=debug,servo=info,servo_media=debug" \
    "/Applications/Mercury Browser.app/Contents/MacOS/mercury-browser"
```

From a source checkout, `make run RUST_LOG="talaria_servo=debug,servo=info,servo_media=debug"` does the same. Look for the first GStreamer error, not the messages that follow it. Every start logs one expected warning about `libgstogg.dylib`, because Ogg is left out; see [Troubleshooting](troubleshooting.md#media-does-not-play).
