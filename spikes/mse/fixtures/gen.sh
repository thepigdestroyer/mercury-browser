#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# Makes the committed fixtures of the GstMSE spike (lane sp-mse): short, synthetic, fragmented MP4 and
# WebM files that every scenario appends through MSE. Run it only to regenerate them; the committed
# bytes and fixtures/SHA256SUMS are the reference (the MP4 muxers write the current time into the
# movie header, so a rerun gives equivalent files, not identical bytes).
#
#   spikes/mse/fixtures/gen.sh [output-directory]       default: this directory
#
# Content: 8.0 s, 25 fps, a key frame every 2 s (50 frames), fragments of 2 s (4 media segments). The
# video background is red for the first 4 s and blue for the last 4 s, with a small black ball, so
# the mean colour of a frame says which half of the clip it came from. The audio is a 440 Hz sine,
# mono, 48 kHz. Each file has one track (as DASH delivers them) except muxed-h264-aac.mp4. The two
# renditions of a codec have the same key frames and fragment boundaries.
#
# Tools: gst-launch-1.0 from the bundled GStreamer (scripts/macos/gstreamer.sh), in a private plugin
# registry in a scratch directory (deleted at the end), with no sound, no window and no network.
# Encoders: openh264enc (BSD), vtenc_h264 (Apple VideoToolbox), voaacenc, vp9enc (libvpx), opusenc,
# svtav1enc (BSD-3-Clause + patent grant); muxers cmafmux and isofmp4mux (gst-plugins-rs, MPL-2.0) and
# webmmux. Never x264enc or x265enc: they are GPL.

set -eu

here="$(cd "$(dirname -- "$0")" && pwd -P)"
root="$(cd "$here/../../.." && pwd -P)"
out="${1:-$here}"

if [ -z "${MERCURY_GSTREAMER_PREFIX:-}" ]; then
    MERCURY_GSTREAMER_PREFIX="$("$root/scripts/macos/gstreamer.sh" prefix)"
fi
if [ ! -x "$MERCURY_GSTREAMER_PREFIX/bin/gst-launch-1.0" ]; then
    printf '%s\n' "gen.sh: no GStreamer in $MERCURY_GSTREAMER_PREFIX; run: make gstreamer-macos" >&2
    exit 1
fi

scratch="${TMPDIR:-/tmp}/opencode/lanes/sp-mse/fixtures.$$"
mkdir -p "$scratch" "$out/fmp4" "$out/webm"
trap 'rm -rf "$scratch"' EXIT INT TERM

# An isolated registry that scans only the plugins this script needs, linked into a scratch directory.
# Scanning the whole framework directory loads libgstvulkan, which on an Intel Mac with an AMD GPU can
# hang inside MoltenVK (vkCreateInstance) for minutes; none of the fixtures needs it.
mkdir -p "$scratch/plugins"
for name in coreelements videoconvertscale videotestsrc audiotestsrc audioconvert audioresample \
    openh264 videoparsersbad audioparsers isobmff voaacenc vpx opus matroska svtav1 applemedia; do
    ln -s "$MERCURY_GSTREAMER_PREFIX/lib/gstreamer-1.0/libgst$name.dylib" "$scratch/plugins/libgst$name.dylib"
done
GST_REGISTRY_1_0="$scratch/registry.bin"
GST_PLUGIN_SYSTEM_PATH_1_0="$scratch/plugins"
GST_PLUGIN_PATH_1_0=""
GST_PLUGIN_SCANNER_1_0="$MERCURY_GSTREAMER_PREFIX/libexec/gstreamer-1.0/gst-plugin-scanner"
export GST_REGISTRY_1_0 GST_PLUGIN_SYSTEM_PATH_1_0 GST_PLUGIN_PATH_1_0 GST_PLUGIN_SCANNER_1_0

RED=4294901760   # 0xFFFF0000, big-endian ARGB
BLUE=4278190335  # 0xFF0000FF
BLACK=4278190080 # 0xFF000000
FRAGMENT=2000000000 # 2 s, in nanoseconds

# gl <gst-launch arguments>: runs gst-launch-1.0 quietly; shows its messages only when it fails.
gl() {
    # stdout goes to /dev/null: svtav1enc prints its banner there.
    if ! "$MERCURY_GSTREAMER_PREFIX/bin/gst-launch-1.0" -q "$@" >/dev/null 2>"$scratch/err"; then
        cat "$scratch/err" >&2
        printf '%s\n' "gen.sh: gst-launch-1.0 failed: $*" >&2
        exit 1
    fi
}

# video_sources <width> <height>: two 4 s sources (100 frames each) feeding the element named c
# (concat): red background, then blue.
video_sources() {
    for color in "$RED" "$BLUE"; do
        printf '%s ' "videotestsrc pattern=ball num-buffers=100 background-color=$color foreground-color=$BLACK ! video/x-raw,width=$1,height=$2,framerate=25/1 ! queue ! c."
    done
}

# 8 s of 440 Hz mono sine at 48 kHz: 375 buffers of 1024 samples.
audio_source() {
    printf '%s' "audiotestsrc wave=sine freq=440 num-buffers=375 samplesperbuffer=1024 ! audio/x-raw,rate=48000,channels=1"
}

# h264 <name> <width> <height> <bitrate in bit/s>: openh264enc, constrained baseline, no B-frames.
h264() {
    # shellcheck disable=SC2046 # the source description is meant to be split into words
    gl concat name=c ! videoconvert ! video/x-raw,format=I420 \
        ! openh264enc bitrate="$4" gop-size=50 rate-control=bitrate complexity=low enable-frame-skip=false multi-thread=1 \
        ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au \
        ! cmafmux fragment-duration=$FRAGMENT ! filesink location="$out/fmp4/$1" \
        $(video_sources "$2" "$3")
}

# vp9 <name> <width> <height> <bitrate in bit/s>: libvpx, profile 0, key frame every 50 frames.
vp9() {
    # shellcheck disable=SC2046
    gl concat name=c ! videoconvert ! video/x-raw,format=I420 \
        ! vp9enc target-bitrate="$4" keyframe-max-dist=50 deadline=1 cpu-used=4 end-usage=vbr threads=1 lag-in-frames=0 auto-alt-ref=false \
        ! webmmux streamable=true ! filesink location="$out/webm/$1" \
        $(video_sources "$2" "$3")
}

# P0
h264 video-h264-low.mp4 320 180 150000
h264 video-h264-high.mp4 640 360 400000
# AAC frames are 1024 samples (21.33 ms) and do not divide 2 s: 94 of them make 2.005 s, so the audio
# fragment duration is set just above that (the muxer closes a fragment before it would exceed it),
# which gives 94 + 94 + 94 + 93 frames: four fragments for the 375 frames.
AUDIO_FRAGMENT=2006000000
# shellcheck disable=SC2046
gl $(audio_source) ! voaacenc bitrate=32000 ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw \
    ! cmafmux fragment-duration=$AUDIO_FRAGMENT ! filesink location="$out/fmp4/audio-aac.mp4"

# P1
# One file with both tracks: isofmp4mux, because cmafmux takes one stream.
# shellcheck disable=SC2046
gl isofmp4mux name=m fragment-duration=$FRAGMENT ! filesink location="$out/fmp4/muxed-h264-aac.mp4" \
    concat name=c ! videoconvert ! video/x-raw,format=I420 \
    ! openh264enc bitrate=150000 gop-size=50 rate-control=bitrate complexity=low enable-frame-skip=false multi-thread=1 \
    ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au ! m. \
    $(video_sources 320 180) \
    $(audio_source) ! voaacenc bitrate=32000 ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw ! m.
vp9 video-vp9-low.webm 320 180 150000
vp9 video-vp9-high.webm 640 360 400000
# Opus frames are 20 ms (960 samples) and the encoder adds one frame of look-ahead at the end, so 399 input
# frames give 400 Opus frames = 8.000 s. webmmux starts a new Cluster every 500 ms in an audio-only file
# (it has no cluster-duration setting), so this file has 16 media segments, not 4.
gl audiotestsrc wave=sine freq=440 num-buffers=399 samplesperbuffer=960 ! audio/x-raw,rate=48000,channels=1 \
    ! opusenc bitrate=24000 ! webmmux streamable=true ! filesink location="$out/webm/audio-opus.webm"
# B-frames: VideoToolbox's H.264 encoder with frame reordering (High profile, ctts offsets).
# shellcheck disable=SC2046
gl concat name=c ! videoconvert ! video/x-raw,format=I420 \
    ! vtenc_h264 realtime=false allow-frame-reordering=true max-keyframe-interval=50 bitrate=400 \
    ! video/x-h264,profile=high ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au \
    ! cmafmux fragment-duration=$FRAGMENT ! filesink location="$out/fmp4/video-h264-bframes.mp4" \
    $(video_sources 640 360)

# P2
# svtav1enc writes no codec_data, so webmmux would write no CodecPrivate, which a WebM AV1 track needs
# (the av1C record). av1parse reports what the encoder produced: profile main, level 2.0, tier main, 8-bit
# 4:2:0, which is the record 81 00 0c 00 (version 1, seq_profile 0 and level index 0, tier 0 and 4:2:0),
# added here as codec_data in a caps filter. rav1enc does write it but has no key frame interval setting.
# svtav1enc also mis-times the frames that follow the switch from the red source to the blue one (concat
# changes the segment there): the Clusters came out with duplicate and jumping timestamps. identity with
# single-segment=true turns the running time into plain timestamps first, which fixes it.
# shellcheck disable=SC2046
gl concat name=c ! identity single-segment=true ! videoconvert ! video/x-raw,format=I420 \
    ! svtav1enc target-bitrate=150 intra-period-length=49 preset=10 \
    ! av1parse ! "video/x-av1,stream-format=(string)obu-stream,alignment=(string)tu,codec_data=(buffer)81000c00" \
    ! webmmux streamable=true ! filesink location="$out/webm/video-av1-low.webm" \
    $(video_sources 320 180)

printf '%s\n' "gen.sh: wrote the fixtures to $out"
