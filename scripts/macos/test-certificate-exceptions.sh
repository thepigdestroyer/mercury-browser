#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# End-to-end check that a certificate exception is bound to the host it was accepted for.
#
#   scripts/macos/test-certificate-exceptions.sh
#
# A loopback HTTPS server with a self-signed certificate (crates/mercury-site-security,
# example tls_test_server) has two names in that certificate, 127.0.0.1 and localhost, and
# both reach it. Its page /host-binding loads an image from the name the page was opened under
# ("same") and from the other name ("other") and sets its title to
# "CERTEXC same=<loaded|failed>; other=<loaded|failed>". The browser is run on it with
# MERCURY_TEST_PROCEED_ON_CERTIFICATE_ERROR=1, which makes Mercury's certificate page continue
# by itself in an automated run, so the exception is made the way a user makes it:
#  - opened as 127.0.0.1: the exception is for (127.0.0.1, this certificate); same=loaded and
#    other=failed. With an engine that accepts the certificate on every host, other=loaded.
#  - opened as localhost: the same the other way round (it shows that other=failed above is the
#    certificate, not a name that does not connect);
#  - without the variable: the certificate page ("Certificate not valid") stays and nothing is
#    accepted.
# Every run must exit 0 and leave nothing running.
#
# Needs a built target/release/mercury-browser (make build-release); it builds the test server
# itself (a light build, without the engine). Never touches a trust store, the keychain or the
# user's profile: each run is an automated --screenshot run with its own temporary profile, and
# the server listens on 127.0.0.1 only. Prints one line per check and exits non-zero if any
# fails. Writes only to a temporary directory, which it removes;
# MERCURY_TEST_CERTEXC_KEEP=<directory> copies the screenshots and logs there first, and
# MERCURY_TEST_CERTEXC_DELAY=<milliseconds> sets how long each run waits before its screenshot
# (default 10000).
set -euo pipefail

cd "$(dirname -- "${BASH_SOURCE[0]}")/../.."
REPO="$PWD"
TARGET="${CARGO_TARGET_DIR:-target}"
case "$TARGET" in
    /*) ;;
    *) TARGET="$REPO/$TARGET" ;;
esac
EXE="$TARGET/release/mercury-browser"
SERVER_EXE="$TARGET/release/examples/tls_test_server"
TMP="$(mktemp -d -t mercury-test-certexc)"
SERVER_PID=""
cleanup() {
    if [ -n "$SERVER_PID" ]; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    if [ -n "${MERCURY_TEST_CERTEXC_KEEP:-}" ]; then
        mkdir -p -- "$MERCURY_TEST_CERTEXC_KEEP"
        find "$TMP" -maxdepth 1 -type f \( -name '*.png' -o -name '*.log' -o -name '*.out' -o -name '*.err' \) \
            -exec cp {} "$MERCURY_TEST_CERTEXC_KEEP/" \;
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
yes_no() { if "$@"; then echo yes; else echo no; fi; }

[ -f "$EXE" ] || { printf 'FAIL %s is missing; run make build-release\n' "$EXE"; exit 1; }

# ── The test server ───────────────────────────────────────────────────────
# Built and started here, so that $! is the server itself. --port 0: the system picks a free
# port, which is the first line of its output.
"${CARGO:-cargo}" build --locked --release --quiet -p mercury-site-security --example tls_test_server \
    || { printf 'FAIL the test server did not build\n'; exit 1; }
"$SERVER_EXE" --port 0 >"$TMP/server.out" 2>"$TMP/server.err" &
SERVER_PID=$!
PORT=""
for _ in $(seq 100); do
    PORT="$(sed -n 's#^listening on https://127\.0\.0\.1:\([0-9][0-9]*\)/$#\1#p' "$TMP/server.out" | head -n 1)"
    [ -n "$PORT" ] && break
    kill -0 "$SERVER_PID" 2>/dev/null || break
    sleep 0.1
done
[ -n "$PORT" ] || { printf 'FAIL the test server did not start\n'; exit 1; }
printf 'INFO the test server listens on 127.0.0.1:%s\n' "$PORT"

# ── Runs ──────────────────────────────────────────────────────────────────
# What was running from the browser's executable before the first run (normally nothing).
running() { { pgrep -f -- "$EXE" || true; } | sort | tr '\n' ' '; }
BASELINE="$(running)"
# yes once nothing that the run started is left (the content processes may take a moment).
settled() {
    for _ in $(seq 100); do
        if [ "$(running)" = "$BASELINE" ]; then
            echo yes
            return 0
        fi
        sleep 0.1
    done
    echo no
}

LOG=""
# Runs the browser on $2 as an automated screenshot run; LOG is the run's log. $3 is yes or no:
# whether the test switch is set (it is removed from the environment when no). Logs: the window
# titles (winit::window) and the exception handler's info line.
run_page() {
    local name="$1" url="$2" proceed="$3" code=0
    local switch=(-u MERCURY_TEST_PROCEED_ON_CERTIFICATE_ERROR)
    if [ "$proceed" = yes ]; then switch=(MERCURY_TEST_PROCEED_ON_CERTIFICATE_ERROR=1); fi
    LOG="$TMP/$name.log"
    env "${switch[@]}" \
        RUST_LOG="warn,winit::window=debug,talaria_servo::certificate_exceptions=info" \
        "$EXE" --screenshot "$TMP/$name.png" --window-size 900x700 \
        --screenshot-delay "${MERCURY_TEST_CERTEXC_DELAY:-10000}" "$url" >"$LOG" 2>&1 || code=$?
    check "$name: the run exits 0" 0 "$code"
    check "$name: the screenshot was saved" yes "$(yes_no test -s "$TMP/$name.png")"
    check "$name: nothing keeps running" yes "$(settled)"
}
# The last window title in LOG that starts with $1, without the application's name.
last_title() {
    { grep -o "title=\"$1[^\"]*" "$LOG" || true; } | tail -n 1 | sed 's/^title="//; s/ — Mercury Browser$//'
}
# How many times the log says that an exception was accepted for host $1.
accepted() { grep -cF -- "accepted an invalid certificate for $1 at the user's request" "$LOG" || true; }
# yes if at least one was.
was_accepted() { if [ "$(accepted "$1")" -gt 0 ]; then echo yes; else echo no; fi; }

# The exception is for 127.0.0.1; the image under the other name must fail.
run_page by-address "https://127.0.0.1:$PORT/host-binding" yes
check "by-address: the page loaded the image under its own name and refused the other" \
    "CERTEXC same=loaded; other=failed" "$(last_title CERTEXC)"
check "by-address: an exception was accepted for 127.0.0.1" yes "$(was_accepted 127.0.0.1)"
check "by-address: none was accepted for localhost" 0 "$(accepted localhost)"

# The same the other way round.
run_page by-name "https://localhost:$PORT/host-binding" yes
check "by-name: the page loaded the image under its own name and refused the other" \
    "CERTEXC same=loaded; other=failed" "$(last_title CERTEXC)"
check "by-name: an exception was accepted for localhost" yes "$(was_accepted localhost)"
check "by-name: none was accepted for 127.0.0.1" 0 "$(accepted 127.0.0.1)"

# Control: without the test switch the certificate page stays and nothing is accepted.
run_page control "https://127.0.0.1:$PORT/host-binding" no
check "control: the certificate page is shown" "Certificate not valid" "$(last_title 'Certificate not valid')"
check "control: the page of the server never ran" "" "$(last_title CERTEXC)"
check "control: no exception was accepted" "0|0" "$(accepted 127.0.0.1)|$(accepted localhost)"

# What the server saw, for the person reading the result: a refusal at the handshake carries
# the name the browser asked for.
if grep -q 'name asked for: localhost' "$TMP/server.err"; then
    printf 'INFO the server saw the browser end a connection for the name localhost: %s\n' \
        "$(grep 'name asked for: localhost' "$TMP/server.err" | head -n 1)"
fi

if [ "$failures" -gt 0 ]; then
    printf '%s check(s) failed. What the runs logged:\n' "$failures"
    for log in "$TMP"/*.log; do
        printf -- '--- %s\n' "${log##*/}"
        grep -E 'title="(CERTEXC|Certificate)|invalid certificate|certificate exception' "$log" | head -n 10 || true
    done
    printf 'Set MERCURY_TEST_CERTEXC_KEEP=<directory> to keep the screenshots and the full logs.\n'
    exit 1
fi
printf 'all checks passed\n'
