/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack

[Exposed=Window]
interface MediaStreamTrack : EventTarget {
    readonly        attribute DOMString kind;
    readonly        attribute DOMString id;
    // readonly        attribute DOMString label;
    //                 attribute boolean enabled;
    // readonly        attribute boolean muted;
    //                 attribute EventHandler onmute;
    //                 attribute EventHandler onunmute;
    //                 attribute EventHandler onended;
    MediaStreamTrack clone();
    // Mercury: readyState and stop(), so that a page can release a camera or microphone.
    readonly        attribute MediaStreamTrackState readyState;
    undefined stop();
    // MediaTrackCapabilities getCapabilities();
    // MediaTrackConstraints getConstraints();
    // MediaTrackSettings getSettings();
    // Promise<void> applyConstraints(optional MediaTrackConstraints constraints);
};

// Mercury: added with readyState.
enum MediaStreamTrackState {
    "live",
    "ended"
};
