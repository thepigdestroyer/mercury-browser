#!/usr/bin/env bash
# End-to-end checks of camera and microphone capture with fake devices only.
#
#   scripts/macos/test-capture.sh      (or: make test-capture-macos)
#
# Never touches a real camera or microphone and never plays a sound:
#  - target/release/mercury-browser is copied into a temporary directory and its
#    GStreamer libraries are collected beside it WITHOUT applemedia (the camera
#    source) and osxaudio (the microphone source and the audio output), and WITH
#    videotestsrc and audiotestsrc (MERCURY_GSTREAMER_EXCLUDE_PLUGINS and
#    MERCURY_GSTREAMER_EXTRA_PLUGINS of collect-gstreamer.sh);
#  - the copy runs with --fake-media-devices, which refuses to start unless its
#    lib directory looks exactly like that, and only with --screenshot (no focus
#    is taken). Its content processes then offer "Mercury fake camera" and
#    "Mercury fake microphone" (a moving test pattern and a sine wave).
# The script checks the copy before running it, and that the normal build
# refuses the mode.
#
# Needs a built target/release/mercury-browser (make build-release) and the
# unpacked GStreamer (make gstreamer-macos). Prints one line per check and exits
# non-zero if any fails. Writes only to a temporary directory, which it removes;
# MERCURY_TEST_CAPTURE_KEEP=<directory> copies the screenshots and logs there first.
set -euo pipefail

cd "$(dirname -- "${BASH_SOURCE[0]}")/../.."
REPO="$PWD"
EXE="$REPO/target/release/mercury-browser"
TMP="$(mktemp -d -t mercury-test-capture)"
SERVER_PID=""
cleanup() {
    if [ -n "$SERVER_PID" ]; then kill "$SERVER_PID" 2>/dev/null || true; fi
    if [ -n "${MERCURY_TEST_CAPTURE_KEEP:-}" ]; then
        mkdir -p -- "$MERCURY_TEST_CAPTURE_KEEP"
        find "$TMP" -maxdepth 1 -type f \( -name '*.png' -o -name '*.log' -o -name '*.out' \) \
            -exec cp {} "$MERCURY_TEST_CAPTURE_KEEP/" \;
    fi
    rm -rf -- "$TMP"
}
trap cleanup EXIT
failures=0

check() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        printf 'PASS %s\n' "$name"
    else
        printf 'FAIL %s: expected [%s], got [%s]\n' "$name" "$expected" "$actual"
        failures=$((failures + 1))
    fi
}

[ -f "$EXE" ] || { printf 'FAIL %s is missing; run make build-release\n' "$EXE"; exit 1; }
gst_env="$(scripts/macos/gstreamer.sh env)" || { printf 'FAIL no GStreamer; run make gstreamer-macos\n'; exit 1; }
eval "$gst_env"

# ── The copy with fake-device plugins only ────────────────────────────────
RUN="$TMP/run"
mkdir -p "$RUN"
cp "$EXE" "$RUN/mercury-browser"
MERCURY_GSTREAMER_EXCLUDE_PLUGINS="ogg applemedia osxaudio" \
    MERCURY_GSTREAMER_EXTRA_PLUGINS="videotestsrc audiotestsrc" \
    scripts/macos/collect-gstreamer.sh "$MERCURY_GSTREAMER_PREFIX" "$RUN/lib" 2>"$TMP/collect.log" \
    || { cat "$TMP/collect.log"; printf 'FAIL collecting the fake-device libraries\n'; exit 1; }
present() { if [ -e "$RUN/lib/$1" ]; then printf yes; else printf no; fi; }
check "the copy has no camera plugin" no "$(present libgstapplemedia.dylib)"
check "the copy has no microphone plugin" no "$(present libgstosxaudio.dylib)"
check "the copy has the test sources" "yes yes" \
    "$(present libgstvideotestsrc.dylib) $(present libgstaudiotestsrc.dylib)"
# Stop here rather than run anything if the copy could reach a real device.
if [ "$(present libgstapplemedia.dylib)$(present libgstosxaudio.dylib)" != nono ]; then
    printf 'FAIL refusing to run: the copy could open a real device\n'
    exit 1
fi

# ── The normal build refuses the mode (it exits before starting the engine) ──
code=0
"$EXE" --fake-media-devices --screenshot "$TMP/never.png" about:blank >"$TMP/refused.out" 2>&1 || code=$?
check "the normal build refuses --fake-media-devices" "2|yes" \
    "$code|$(if grep -q "fake-media-devices refused: .*libgstapplemedia.dylib" "$TMP/refused.out"; then echo yes; else echo no; fi)"

# ── Pages ─────────────────────────────────────────────────────────────────
python3 scripts/serve-tests.py >"$TMP/port" 2>"$TMP/server.log" &
SERVER_PID=$!
for _ in $(seq 50); do [ -s "$TMP/port" ] && break; sleep 0.1; done
PORT="$(head -n 1 "$TMP/port")"
[ -n "$PORT" ] || { printf 'FAIL the test server did not start\n'; exit 1; }

# Runs the copy on a page; sets TITLE to the last "CAPTURE …" window title and LOG to the log.
# Extra arguments go to the browser (for example --answer-permission-prompts allow).
TITLE=""
LOG=""
run_page() {
    local name="$1" url="$2"
    shift 2
    LOG="$TMP/$name.log"
    local code=0
    # The fake devices write a line when the engine opens one. GST_PLUGIN_LOADING: the control
    # that the content process's output reaches the file (the fake-device set-up loads the test
    # sources). GST_PARENTAGE: a stopped source is taken out of its pipeline. The page's own
    # work can take a while on a busy machine, hence the delay.
    RUST_LOG="warn,winit::window=debug,mercury_browser::media_capture=info,mercury_browser::screenshot=info" \
        GST_DEBUG="GST_PLUGIN_LOADING:4,GST_PARENTAGE:4" \
        "$RUN/mercury-browser" --fake-media-devices --screenshot "$TMP/$name.png" \
        --window-size 900x700 --screenshot-delay "${MERCURY_TEST_CAPTURE_DELAY:-15000}" \
        "$@" "$url" >"$LOG" 2>&1 || code=$?
    TITLE="$(grep -o 'title="CAPTURE [^"]*' "$LOG" | tail -n 1 | sed 's/^title="//; s/ — Mercury Browser$//')"
    check "$name: the run exits 0" 0 "$code"
}
field() { # key → value from TITLE ("CAPTURE k=v; k=v")
    tr ';' '\n' <<<"${TITLE#CAPTURE }" | sed -n "s/^ *$1=//p" | head -n 1
}
yes_no() { if "$@"; then echo yes; else echo no; fi; }
sources_created() { yes_no grep -q 'fake media devices: the engine opened the fake' "$LOG"; }
prompt_answered() { yes_no grep -q 'answering permission prompt' "$LOG"; }
# Whether the camera source left its pipeline before the screenshot was saved: both processes
# write to the same log in order, and the browser shuts down (which releases everything anyway)
# only after the screenshot, so a release the page caused comes first.
released_before_shutdown() {
    local removed saved
    removed="$(grep -n 'removed child "videotestsrc' "$LOG" | head -n 1 | cut -d: -f1)"
    saved="$(grep -n 'saved a .* screenshot' "$LOG" | head -n 1 | cut -d: -f1)"
    if [ -n "$removed" ] && [ -n "$saved" ] && [ "$removed" -lt "$saved" ]; then
        echo yes
    else
        echo no
    fi
}

BASE="http://127.0.0.1:$PORT/pages"
FRAME_BASE="http://127.0.0.1:$PORT/pages"

# No answer: the engine's default is "deny", but the prompt stays on screen until the run ends.
run_page unanswered "$BASE/capture.html"
check "unanswered: the fake devices are registered" yes \
    "$(yes_no grep -q 'fake media devices registered' "$LOG")"
check "unanswered: GStreamer's log is captured (control)" yes \
    "$(yes_no eval "grep 'GST_PLUGIN_LOADING' \"\$LOG\" | grep -q 'libgstvideotestsrc.dylib'")"
check "unanswered: navigator.mediaDevices exists" object "$(field mediaDevices)"
check "unanswered: RTCPeerConnection is hidden" undefined "$(field RTCPeerConnection)"
check "unanswered: RTCIceCandidate is hidden" undefined "$(field RTCIceCandidate)"
check "unanswered: before permission, one input of each kind without id or label" \
    "audioinput:-:-,videoinput:-:-" "$(field before)"
check "unanswered: getUserMedia waits for the answer" "asked|" "$(field permission)|$(field getUserMedia)"
check "unanswered: no device source is created before an answer" no "$(sources_created)"

run_page allow "$BASE/capture.html" --answer-permission-prompts allow
check "allow: one prompt was answered for both kinds" 1 "$(grep -c 'answering permission prompt' "$LOG")"
check "allow: the engine opened both fake devices" "1|1" \
    "$(grep -c 'opened the fake camera' "$LOG")|$(grep -c 'opened the fake microphone' "$LOG")"
check "allow: getUserMedia gives a video and an audio track" "ok|video,audio|live,live" \
    "$(field getUserMedia)|$(field tracks)|$(field readyState)"
check "allow: device names are shown after permission" \
    "videoinput:Mercury fake camera:id,audioinput:Mercury fake microphone:id" "$(field after)"
check "allow: the fake camera's frames move" "PASS" "$(field verdict)"
check "allow: stop() ends both tracks" "ended,ended" "$(field stopped)"
check "allow: stop() releases the source before the browser shuts down" yes "$(released_before_shutdown)"

run_page deny "$BASE/capture.html" --answer-permission-prompts deny
check "deny: getUserMedia is refused with NotAllowedError" NotAllowedError "$(field getUserMedia)"
check "deny: no device source was created" no "$(sources_created)"

run_page leave "$BASE/capture.html?then=/pages/capture-after.html" --answer-permission-prompts allow
check "leave: the page left with its devices open" after "$(field verdict)"
check "leave: leaving the page releases the source" yes "$(released_before_shutdown)"

run_page cross-frame "$FRAME_BASE/capture-frame.html?frame=cross" --answer-permission-prompts allow
check "cross-frame: a frame of another origin is refused" "denied|NotAllowedError" \
    "$(field permission)|$(field getUserMedia)"
check "cross-frame: without a prompt" no "$(prompt_answered)"

run_page same-frame "$FRAME_BASE/capture-frame.html?frame=same" --answer-permission-prompts allow
check "same-frame: a frame of the page's own origin may ask" "yes|ok" \
    "$(prompt_answered)|$(field getUserMedia)"

run_page system-denied "$BASE/capture.html" --answer-permission-prompts allow --fake-system-denies-capture
check "system-denied: macOS's refusal refuses the page" NotAllowedError "$(field getUserMedia)"
check "system-denied: no device source was created" no "$(sources_created)"

run_page system-denied-notice "$BASE/capture.html" --fake-system-denies-capture

if [ "$failures" -gt 0 ]; then
    printf '%s check(s) failed (logs were in %s, now removed)\n' "$failures" "$TMP"
    exit 1
fi
printf 'all checks passed\n'
