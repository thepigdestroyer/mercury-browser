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

Also not available: DRM-protected video (Encrypted Media Extensions), picture-in-picture and video calls (WebRTC). On macOS pages can ask to use your camera and microphone; see [Camera and microphone](camera-and-microphone.md). The tab audio indicator, muting a tab and macOS Now Playing are available; see below.

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

## Which tab is playing, and muting a tab

A tab whose page plays sound shows a speaker on its tab; click the speaker, or press ++cmd+shift+m++
on macOS (++ctrl+m++ elsewhere), to mute the tab, and again to unmute it. A muted tab shows a
crossed-out speaker for as long as it stays muted, including after it goes to another page.

How Mercury decides that a tab makes sound:

- Servo reports when the page's `<audio>` and `<video>` elements play or pause. Mercury then looks
  into the page about once a second while it plays. If everything it can see playing is muted (or
  at volume 0), and the page has no frames, the tab shows no speaker; this hides the silent
  background videos many sites use.
- Whatever Mercury cannot see counts as sound, so that a tab that makes sound is never hidden: media
  in frames (embedded players, but also any page with an advertising frame), in closed shadow
  roots, or in elements outside the page that started before Mercury first looked. Such pages can
  show a speaker while silent.
- Sound made only with Web Audio (`AudioContext`) is not shown.

Muting is best effort, and the speaker's label says so. Servo has no engine-level mute yet, so
Mercury mutes the page's media elements from inside the page: elements already there, those added
later, those that play while outside the page, and those in open shadow roots it finds, and it
suspends audio contexts created after the first mute. About once a second it checks again and mutes
anything the page un-muted.

- A page can undo the mute between checks, and media in frames from other sites, audio contexts
  created before the mute, closed shadow roots and the first moments of media that starts as a
  page opens are not muted.
- The page can notice: it receives `volumechange` events, sees `muted` set on its elements, and
  sees a helper property on its window, whose name is random for each run and different for every site, so sites cannot use it to recognise each other (and a wrapped `AudioContext` and
  `HTMLMediaElement.prototype.play`), which a script could use to tell that Mercury looked or muted.

## Media keys and Now Playing (macOS)

The tab that most recently started playing sound appears in Control Centre's Now Playing. It shows
the page's title, artist and album, cut to 256 characters and cleaned of control characters. It
also always names the page's site: as the artist when the page gives none, otherwise after the
album. The play/pause key (and AirPods controls) acts on that tab. Servo has one media session
for all tabs, so while another tab also plays, play and pause are done inside the page and next and
previous are not offered; when that tab plays alone, they go through Servo and reach pages that
handle them through the Media Session API.

The tab stays in Now Playing while paused, so play resumes it. It is removed when the tab stops,
goes to another page, is muted or is closed. A tab that pauses and plays again within a second (as
pages with several media elements do) does not take Now Playing back from another tab. Muted tabs
and silent background videos (even beside a frame) never take the media keys.

While the active tab plays a video in fullscreen, Mercury keeps the display from going to sleep;
pausing, leaving fullscreen, switching tabs or quitting lets it sleep again.

## Diagnose playback

Start Mercury from Terminal with media logging:

```bash
RUST_LOG="talaria_servo=debug,servo=info,servo_media=debug" \
    "/Applications/Mercury Browser.app/Contents/MacOS/mercury-browser"
```

From a source checkout, `make run RUST_LOG="talaria_servo=debug,servo=info,servo_media=debug"` does the same. Look for the first GStreamer error, not the messages that follow it. Every start logs one expected warning about `libgstogg.dylib`, because Ogg is left out; see [Troubleshooting](troubleshooting.md#media-does-not-play).
