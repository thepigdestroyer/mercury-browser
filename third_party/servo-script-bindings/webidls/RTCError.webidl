/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// https://w3c.github.io/webrtc-pc/#dom-rtcerror

// Mercury: WebRTC is gated by dom_webrtc_transceiver_enabled, not dom_webrtc_enabled,
// so that dom_webrtc_enabled exposes camera and microphone capture only (third_party/servo-script-bindings/MERCURY.md).
[Exposed=Window, Pref="dom_webrtc_transceiver_enabled"]
interface RTCError : DOMException {
  constructor(RTCErrorInit init, optional DOMString message = "");
  readonly attribute RTCErrorDetailType errorDetail;
  readonly attribute long? sdpLineNumber;
  readonly attribute long? httpRequestStatusCode;
  readonly attribute long? sctpCauseCode;
  readonly attribute unsigned long? receivedAlert;
  readonly attribute unsigned long? sentAlert;
};

dictionary RTCErrorInit {
  required RTCErrorDetailType errorDetail;
  long sdpLineNumber;
  long httpRequestStatusCode;
  long sctpCauseCode;
  unsigned long receivedAlert;
  unsigned long sentAlert;
};

enum RTCErrorDetailType {
  "data-channel-failure",
  "dtls-failure",
  "fingerprint-failure",
  "sctp-failure",
  "sdp-syntax-error",
  "hardware-encoder-not-available",
  "hardware-encoder-error"
};
