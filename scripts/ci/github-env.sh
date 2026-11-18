#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Run `scripts/macos/gstreamer.sh env` (or another command printing the same
# `export` lines) and hand the variables it sets to the later steps of a
# GitHub Actions job.
#
#   scripts/ci/github-env.sh [--print] <command> [arguments…]
#   scripts/ci/github-env.sh scripts/macos/gstreamer.sh env
#
#   --print   write the entries to standard output instead of $GITHUB_ENV and
#             $GITHUB_PATH (to see what would be exported)
#
# The command's output is evaluated by bash exactly as
# `eval "$(<command>)"` would be in a shell, so quoting, `$VAR` expansion and
# path lists such as `PATH=/x/bin:$PATH` mean what they mean there; nothing is
# parsed by hand. Only these variables may change, and the first four must be
# set:
#
#   MERCURY_GSTREAMER_PREFIX  the unpacked GStreamer prefix
#   PKG_CONFIG                its pkg-config
#   PKG_CONFIG_LIBDIR         its .pc directory
#   PATH                      the old PATH with directories added in front;
#                             they go to $GITHUB_PATH
#   PKG_CONFIG_PATH           may only be unset; exported as empty, which
#                             pkg-config treats the same
#
# Each value is written with GitHub's multi-line form (NAME<<DELIMITER, value,
# DELIMITER) using a random delimiter checked not to occur in the value, so
# spaces, colons, quotes and newlines arrive unchanged.
#
# Refuses, exporting nothing, if the command fails, sets or unsets anything
# else, leaves one of the four unset, or changes PATH in any way other than
# adding directories in front. DYLD_* variables are refused with an
# explanation: macOS strips them whenever a SIP-protected program such as
# /bin/bash starts, so they could not reach any later command.
#
# The comparison is `github-env` (Rust, crates/mercury-devtools), built in
# release mode before the command runs; cargo's messages go to standard error.
set -euo pipefail

usage() {
    sed -n '/^# Run `scripts\/macos/,/^# \/bin\/bash starts/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

PRINT=()
case "${1:-}" in
    --print) PRINT=(--print); shift ;;
    -h | --help) usage; exit 0 ;;
esac
[ "$#" -gt 0 ] || { usage >&2; exit 2; }
if [ "${#PRINT[@]}" = 0 ]; then
    [ -n "${GITHUB_ENV:-}" ] && [ -n "${GITHUB_PATH:-}" ] ||
        { printf 'github-env: GITHUB_ENV and GITHUB_PATH are not set (use --print outside Actions)\n' >&2; exit 2; }
fi

REPOSITORY="$(cd "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
devtools() {
    "${CARGO:-cargo}" "$1" --locked --release --quiet --manifest-path "$REPOSITORY/Cargo.toml" \
        --package mercury-devtools --bin github-env "${@:2}"
}
# Built first, so that a build failure stops here before the command runs.
devtools build >&2

BEFORE="$(mktemp)"
AFTER="$(mktemp)"
trap 'rm -f -- "$BEFORE" "$AFTER"' EXIT

# DYLD_* variables, listed by bash itself: /usr/bin/env is SIP-protected and
# would never see them.
dyld_variables() {
    local name
    for name in $(compgen -e); do
        case "$name" in
            DYLD_*) printf '%s=%s\n' "$name" "${!name}" ;;
        esac
    done
}

# Snapshot the environment, run the command (its failure stops the script
# through set -e), then evaluate its output exactly as a shell would and
# snapshot the result. The evaluation happens in a subshell, so that cargo,
# below, runs with the environment this script started with.
/usr/bin/env -0 >"$BEFORE"
DYLD_BEFORE="$(dyld_variables)"
OUTPUT="$("$@")"
(
    eval "$OUTPUT"
    DYLD_AFTER="$(dyld_variables)"
    if [ "$DYLD_AFTER" != "$DYLD_BEFORE" ]; then
        names="$(printf '%s\n' "$DYLD_AFTER" | sed -n 's/=.*//p' | tr '\n' ' ')"
        printf 'github-env: refused: %s(macOS strips DYLD_* variables when a SIP-protected shell starts, so they cannot pass between steps; set them on the command line that needs them)\n' "$names" >&2
        exit 1
    fi
    /usr/bin/env -0 >"$AFTER"
)

devtools run -- "$BEFORE" "$AFTER" ${PRINT[@]+"${PRINT[@]}"}
