# Patched servo-script-bindings 0.8.0

This directory vendors `servo-script-bindings` 0.8.0, Servo's WebIDL files and
binding generator, under its original licences: `license = "MPL-2.0 AND
BSD-3-Clause"` — the crate's own code and the WebIDL parser are MPL-2.0 (file
headers); the bundled PLY (`third_party/ply`) is BSD-3-Clause (licence text in
`third_party/ply/ply/lex.py` and `yacc.py`). It is the crate exactly as
published on crates.io (checksum
`75f9bbcb6eb03e96610c1cbd9e71a26869874178e4f109a9a1c3f810bce88f81`), built from
https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/script_bindings`,
added unchanged in commit `ea292e9`.

It is part of the media-capture patch described in
`third_party/servo-script/MERCURY.md`. Every change is marked `Mercury:`.

## Changes

1. **`webidls/MediaStreamTrack.webidl`:** `readonly attribute
   MediaStreamTrackState readyState`, `undefined stop()` and the
   `MediaStreamTrackState` enum (implemented in `servo-script`).
2. **`webidls/MediaDevices.webidl`:** `navigator.mediaDevices` is
   `[SecureContext]`, as the interface already was (as published, the
   attribute carried only the preference).
3. **`webidls/RTC*.webidl` (nine files):** WebRTC's interfaces are gated by
   `dom_webrtc_transceiver_enabled` instead of `dom_webrtc_enabled`, so that
   `dom_webrtc_enabled` exposes camera and microphone capture only. Mercury
   keeps `dom_webrtc_transceiver_enabled` off: Servo 0.8's WebRTC hands every
   local IP address to the page, lacks `addTrack`, TURN and the GStreamer SRTP
   and SCTP plugins (R8 §4). The preference already gated
   `RTCRtpTransceiver` and `RTCRtpSender`; no new preference was added because
   an unknown preference name panics and adding one means patching
   `servo-config`.

Drop when: Servo implements `stop()`/`readyState` and Mercury enables WebRTC
or Servo gives capture its own preference.
