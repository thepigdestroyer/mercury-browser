/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: (MED-1) the Media Source Extensions DOM: `MediaSource`, `SourceBuffer` and
// `SourceBufferList`. They are the JavaScript-visible objects, their events and the task queuing; every
// spec algorithm that does not need a DOM object (readyState, duration, endOfStream, the buffered
// intersection, abort, remove, changeType, the append window, the timestamp offset, the mode, `updating`)
// is a call into `MediaSourceBackend` (crates/mercury-mse), which the media player feeds from. The
// structure, the object URL store and the event plumbing are carried from servo/servo#47269 by
// decomoraes (MPL-2.0); see `third_party/servo-script/MERCURY.md`.
//
// A new directory (not `dom/media/`) so that `dom/media/mod.rs` stays upstream's.

pub(crate) mod mediasource;
pub(crate) mod sourcebuffer;
pub(crate) mod sourcebufferlist;
// Mercury: (MED-1b) the underrun state, the track lists' contents and `getVideoPlaybackQuality`.
pub(crate) mod stall;
pub(crate) mod tracks;
pub(crate) mod videoplaybackquality;
