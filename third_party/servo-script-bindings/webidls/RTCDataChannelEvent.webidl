/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// https://w3c.github.io/webrtc-pc/#dom-rtcdatachannelevent

// Mercury: WebRTC is gated by dom_webrtc_transceiver_enabled, not dom_webrtc_enabled,
// so that dom_webrtc_enabled exposes camera and microphone capture only (third_party/servo-script-bindings/MERCURY.md).
[Exposed=Window, Pref="dom_webrtc_transceiver_enabled"]
interface RTCDataChannelEvent : Event {
  constructor(DOMString type, RTCDataChannelEventInit eventInitDict);
  readonly attribute RTCDataChannel channel;
};

dictionary RTCDataChannelEventInit : EventInit {
  required RTCDataChannel channel;
};
