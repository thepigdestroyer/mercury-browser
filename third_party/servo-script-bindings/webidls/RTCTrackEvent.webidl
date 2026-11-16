/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// https://w3c.github.io/webrtc-pc/#dom-rtctrackevent

// Mercury: WebRTC is gated by dom_webrtc_transceiver_enabled, not dom_webrtc_enabled,
// so that dom_webrtc_enabled exposes camera and microphone capture only (third_party/MERCURY.md).
[Exposed=Window, Pref="dom_webrtc_transceiver_enabled"]
interface RTCTrackEvent : Event {
    [Throws] constructor(DOMString type, RTCTrackEventInit eventInitDict);
    // readonly attribute RTCRtpReceiver           receiver;
    readonly attribute MediaStreamTrack         track;
    // [SameObject]
    // readonly attribute FrozenArray<MediaStream> streams;
    // readonly attribute RTCRtpTransceiver        transceiver;
};

// https://www.w3.org/TR/webrtc/#dom-rtctrackeventinit
dictionary RTCTrackEventInit : EventInit {
    // required RTCRtpReceiver        receiver;
    required MediaStreamTrack      track;
             // sequence<MediaStream> streams = [];
    // required RTCRtpTransceiver     transceiver;
};
