#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# A stub browser for the tests and the dry run of `mse-run` (docs/developer-guide/mse-testing.md): it
# takes the arguments of the real browser (the last one is the URL), posts canned reports to
# /__report of the server named in the URL, and then stays alive until it is killed, as a browser does.
# It starts no window and loads no page.
#
#   mse-run all --no-lock-check --browser crates/mercury-devtools/tests/stub-browser.sh
#
# Without STUB_CANNED every page passes with two PASS lines and every WPT file finishes OK with one
# passing subtest. With STUB_CANNED=<dir>, a file in it named like the URL path with slashes turned
# into underscores (`pages_mse-basic.html`, `media-source_mediasource-duration.html`) says what to
# do instead:
#   first line #crash      exit at once, before any report (the browser died)
#   first line #silent     say nothing and wait (the page never loaded)
#   first line #stubborn   say nothing, ignore SIGTERM and wait (to be killed with SIGKILL)
#   first line #hang       post "hello" and wait (the page started and never finished)
#   anything else          every line is one JSON report to post; @TOKEN@ is the run's token
# With STUB_LOG=<file>, the URL path of every start is appended to it.
set -eu

url=""
for argument in "$@"; do
    url="$argument"
done
case "$url" in
    http://*) ;;
    *) printf 'stub-browser: the last argument must be an http URL, got %s\n' "$url" >&2; exit 2 ;;
esac
rest="${url#http://}"
hostport="${rest%%/*}"
path_and_query="/${rest#*/}"
path="${path_and_query%%\?*}"
query=""
case "$path_and_query" in
    *\?*) query="${path_and_query#*\?}" ;;
    *) ;;
esac
token=""
for pair in $(printf '%s' "$query" | tr '&' ' '); do
    case "$pair" in
        __mse_run=*) token="${pair#__mse_run=}" ;;
        *) ;;
    esac
done

if [ -n "${STUB_LOG:-}" ]; then
    printf '%s\n' "$path" >>"$STUB_LOG"
fi

post() {
    printf '%s' "$1" | curl --silent --output /dev/null --max-time 10 --request POST \
        --header 'Content-Type: application/json' --data-binary @- "http://$hostport/__report" || true
}

wait_forever() {
    while :; do
        sleep 1
    done
}

key="$(printf '%s' "${path#/}" | tr '/' '_')"
canned="${STUB_CANNED:-}/$key"
if [ -n "${STUB_CANNED:-}" ] && [ -f "$canned" ]; then
    first="$(sed -n 1p "$canned")"
    case "$first" in
        '#crash') exit 3 ;;
        '#silent') wait_forever ;;
        '#stubborn') trap '' TERM; wait_forever ;;
        '#hang')
            post "{\"token\":\"$token\",\"kind\":\"hello\",\"title\":\"\"}"
            wait_forever
            ;;
        *)
            while IFS= read -r line; do
                [ -n "$line" ] || continue
                post "$(printf '%s' "$line" | sed "s/@TOKEN@/$token/g")"
            done <"$canned"
            exec sleep 60
            ;;
    esac
fi

post "{\"token\":\"$token\",\"kind\":\"hello\",\"title\":\"\"}"
case "$path" in
    /pages/*)
        post "{\"token\":\"$token\",\"kind\":\"page\",\"title\":\"stub: 2 PASS, 0 FAIL (done)\",\"lines\":[\"PASS stub one\",\"PASS stub two\"],\"errors\":[]}"
        ;;
    *)
        post "{\"token\":\"$token\",\"kind\":\"wpt\",\"title\":\"stub\",\"harness\":{\"status\":0,\"message\":null},\"tests\":[{\"name\":\"stub\",\"status\":0,\"message\":null}]}"
        ;;
esac
exec sleep 60
