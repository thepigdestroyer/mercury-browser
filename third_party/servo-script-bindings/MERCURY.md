# Patched servo-script-bindings 0.8.0

This directory vendors `servo-script-bindings` 0.8.0, Servo's WebIDL files and
binding generator, under its original licences: `license = "MPL-2.0 AND
BSD-3-Clause"` — the crate's own code and the WebIDL parser are MPL-2.0 (file
headers); the bundled PLY (`third_party/ply`) is BSD-3-Clause (licence text in
`third_party/ply/ply/lex.py` and `yacc.py`). It is the crate exactly as
published on crates.io (checksum
`75f9bbcb6eb03e96610c1cbd9e71a26869874178e4f109a9a1c3f810bce88f81`), built from
https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/script_bindings`. `third_party/patches/servo-script-bindings.patch` is the difference from the
published crate, and `make servo-patches-check` fails if the two disagree.

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

### net-1 (21 Nov 2026)

- **`webidls/Navigator.webidl`, `webidls/WorkerNavigator.webidl`:** the `NavigatorGPC` mixin (`readonly attribute boolean
  globalPrivacyControl`, no `Pref`), included by both interfaces (EC25).
- **`webidls/WebLock.webidl` (new):** `LockManager`, `Lock`, `LockOptions`, `LockInfo`, `LockManagerSnapshot` and the callback, all
  behind `Pref="dom_web_locks_enabled"` (ED4, servo/servo#47981; the draft has no `Pref`).
- **`webidls/IDBIndex.webidl`:** `get`, `getKey`, `getAll`, `getAllKeys` and `count` uncommented (`[NewObject, Throws]`).
  **`webidls/IDBRequest.webidl`:** `source` is `(IDBObjectStore or IDBIndex or IDBCursor)?` (ED3).
- **`codegen/Bindings.conf`:** the `IDBIndex` `cx` list gains the five methods, `LockManager` gets a `realm` list (`Request`, `Request_`, `Query`) and the two navigators' `cx` lists gain `Locks`.
- Tested by compiling `servo-script`; drop with the corresponding `servo-script` changes (12 to 15).

### K0 (19 Nov 2026)

4. **servo/servo#49649** (`10c279fae`, "Rename `DomSlice` `r()` to `as_ref_slice()`"): `root.rs` (the `DomSlice` trait and its
   impl for `[Dom<T>]`) and `codegen/codegen.py` (`process_arg`: a variadic or sequence interface argument is passed as
   `.as_ref_slice()` in the generated glue; vendored Python edited by one line, as upstream did). The call sites are listed in
   `third_party/servo-script/MERCURY.md`, change 8. Drop at the Servo bump that contains it (B1).
5. **SpiderMonkey 153.7.0:** the `js` dependency (package `mozjs`) is `=0.27.0` instead of `=0.26.13` in `Cargo.toml`
   (servo/servo#49907); see `third_party/servo-script-webcrypto/MERCURY.md`. Drop at B1.

### MED-1: Media Source Extensions (20 Nov 2026)

6. **WebIDL for Media Source Extensions**, carried from servo/servo#47269 by decomoraes (MPL-2.0; that pull request was closed unmerged on 16 Aug 2026 after its author said it was written with an LLM, so Servo never reviewed it: the IDL was compared with the W3C WebIDL), behind
   `dom_media_source_enabled` (default off): new `webidls/MediaSource.webidl` (the constructor may throw: a
   backend that cannot play MSE refuses it), `webidls/SourceBuffer.webidl` (without `textTracks`: this Servo's
   `TextTrackList` is bound to a media element), `webidls/SourceBufferList.webidl`; `webidls/URL.webidl` gains
   `createObjectURL(MediaSource)`; `webidls/HTMLMediaElement.webidl` puts `MediaSource` in the `MediaProvider`
   union (`MediaStreamOrMediaSourceOrBlob`, `srcObject = mediaSource`). `codegen/Bindings.conf`: `cx` method lists for
   `MediaSource` (`AddSourceBuffer`, `EndOfStream`, `RemoveSourceBuffer`) and `SourceBuffer` (`GetBuffered`), and
   `weakReferenceable` for `MediaSource` (the backend's event callback holds a weak reference). The Rust side is
   `third_party/servo-script/dom/mse/`; see `third_party/servo-script/MERCURY.md`. Drop when a Servo release has
   its own MSE (servo/servo#44403).
