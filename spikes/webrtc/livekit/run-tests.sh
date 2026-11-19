#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Runs t01 to t13 of this crate, each test in its own process, so that a libwebrtc CHECK failure
# (which aborts the process and cannot be caught) is reported as that test's failure and does not
# hide the results of the tests after it. Prints every `RESULT tNN ...` line, and a failing line for
# a test that ended without one. Needs libwebrtc and GStreamer in the environment (webrtc.sh env libwebrtc,
# gstreamer.sh env), prepare.sh done, and an Apple Silicon Mac.
#
#   ./run-tests.sh [cargo test options, e.g. --release --target aarch64-apple-darwin]
#
# Without options it runs `cargo test --release`. Exit status 1 when any test did not pass.
set -u

here="$(cd "$(dirname "$0")" && pwd)"
cd "$here" || exit 1
if [ "$#" -eq 0 ]; then
    set -- --release
fi
# GStreamer for t05, t07 and t10: the plugin directory and the libraries, set here because macOS
# removes DYLD_* variables when a SIP-protected shell starts this script.
if [ -n "${MERCURY_GSTREAMER_PREFIX:-}" ]; then
    DYLD_FALLBACK_LIBRARY_PATH="$MERCURY_GSTREAMER_PREFIX/lib"
    GST_PLUGIN_SYSTEM_PATH_1_0="$MERCURY_GSTREAMER_PREFIX/lib/gstreamer-1.0"
    export DYLD_FALLBACK_LIBRARY_PATH GST_PLUGIN_SYSTEM_PATH_1_0
fi
log="$(mktemp "${TMPDIR:-/tmp}/mercury-livekit-tests.XXXXXX")"
trap 'rm -f "$log"' EXIT INT TERM HUP

failures=0
for name in \
    t01_two_peers_connect t02_data_channel_echo t03_stats_shape t04_audio_tone t05_video_from_the_bridge \
    t06_simulcast t07_teardown t08_candidate_policy t09_codecs t10_coexists_with_gstreamer \
    t11_factory_churn t12_build_refuses_without_the_pinned_directory t13_bridge_microphone; do
    id="$(printf '%s' "$name" | cut -c1-3)"
    cargo test "$@" --test calls "$name" -- --exact --nocapture --test-threads=1 >"$log" 2>&1
    status=$?
    # The whole output of each test (INFO lines, panics) goes to RUN_TESTS_FULL_LOG when it is set.
    if [ -n "${RUN_TESTS_FULL_LOG:-}" ]; then cat "$log" >>"$RUN_TESTS_FULL_LOG"; fi
    if grep -o 'RESULT t[0-9][0-9] .*' "$log"; then
        grep -q "RESULT ${id} pass\|RESULT ${id} skip" "$log" || failures=$((failures + 1))
        if [ "$status" -ne 0 ] && grep -q "RESULT ${id} pass" "$log"; then
            printf 'RESULT %s fail the test printed pass but its process ended with status %s (a crash while shutting down)\n' "$id" "$status"
            failures=$((failures + 1))
        fi
    else
        printf 'RESULT %s fail the test process ended with status %s and printed no RESULT line (a libwebrtc abort or a build error); last output: %s\n' \
            "$id" "$status" "$(tail -n 3 "$log" | tr '\n' ' ')"
        failures=$((failures + 1))
    fi
done
[ "$failures" -eq 0 ]
