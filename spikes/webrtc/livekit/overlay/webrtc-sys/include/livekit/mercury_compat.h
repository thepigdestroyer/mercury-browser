// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Declarations that LiveKit's glue (webrtc-sys 0.3.54) takes from its libwebrtc fork
// (webrtc-sdk/webrtc) and that upstream libwebrtc M150, which Shiguredo builds, does not have.
// Copied into vendor/webrtc-sys/include/livekit/ by spikes/webrtc/livekit/prepare.sh.
#pragma once

namespace webrtc {

// The fork adds this callback interface to api/audio/audio_device.h (the platform audio device
// reports a changed device list through it). The glue only stores and forwards the pointer; nothing
// in the spike calls it, and the synthetic audio device the factory creates never reports devices.
class AudioDeviceObserver {
 public:
  virtual ~AudioDeviceObserver() = default;
};

}  // namespace webrtc
