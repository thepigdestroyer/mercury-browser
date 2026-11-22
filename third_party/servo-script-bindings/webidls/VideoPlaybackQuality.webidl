// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// Mercury: (MED-1b) written by Mercury from the Media Playback Quality specification (lane med-1b,
// decision D5), behind `dom_media_source_enabled` like the rest of Media Source Extensions.
// https://w3c.github.io/media-playback-quality/#videoplaybackquality

[Exposed=Window, Pref="dom_media_source_enabled"]
interface VideoPlaybackQuality {
  readonly attribute DOMHighResTimeStamp creationTime;
  readonly attribute unsigned long totalVideoFrames;
  readonly attribute unsigned long droppedVideoFrames;
  readonly attribute unsigned long corruptedVideoFrames;
};
