#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# Fetch the third-party test material of the MSE window run (docs/developer-guide/mse-testing.md):
#
#   - web-platform-tests (BSD-3-Clause), a sparse checkout at a pinned commit: media-source/ and
#     what its tests import (resources/, common/, interfaces/ for the IDL test);
#   - dash.js (BSD-3-Clause), Shaka Player (Apache-2.0) and hls.js (Apache-2.0), one browser build
#     of each at a pinned version, each checked against a pinned SHA-256.
#
#   scripts/dev/fetch-mse-tools.sh            fetch what is missing or wrong, then check
#   scripts/dev/fetch-mse-tools.sh --check    check only, no network
#   scripts/dev/fetch-mse-tools.sh -h
#
# Everything goes into $MSE_TOOLS_DIR (default ~/Library/Caches/mercury-browser/ci-tools/mse on
# macOS, ${XDG_CACHE_HOME:-~/.cache}/mercury-browser/ci-tools/mse elsewhere): the one cache directory
# the disk-guard plugin keeps. Nothing is committed to the repository, so no third-party licence text
# enters it and nothing is redistributed. A download whose SHA-256 differs from the pin below is
# refused and deleted; a pin is changed only by editing this file.
#
# Layout:  $MSE_TOOLS_DIR/wpt/                       the sparse checkout
#          $MSE_TOOLS_DIR/players/<file>             the three player builds
#          $MSE_TOOLS_DIR/VERSIONS                   what was fetched, for the report
# Exit status: 0 everything present and verified, 1 something missing or wrong, 2 usage.
set -eu

# ---- the pins --------------------------------------------------------------------------------
# web-platform-tests master on 2026-11-22 (commit date 2026-11-22T00:22:35Z).
WPT_URL="https://github.com/web-platform-tests/wpt"
WPT_COMMIT="42e27408034f57afe4c726476d1a0bd96b873129"
WPT_DIRS="media-source resources common interfaces"

# name|file|url|sha256 (one browser build of each; jsDelivr serves the npm package files unchanged)
PLAYERS="
dash.js 5.3.0|dash.all.min.js|https://cdn.jsdelivr.net/npm/dashjs@5.3.0/dist/modern/umd/dash.all.min.js|dfd9f5cb0e3d5372d3e5eec709bdda644f21c381af4a01be2f7a62439d01e3db
Shaka Player 5.3.6|shaka-player.compiled.js|https://cdn.jsdelivr.net/npm/shaka-player@5.3.6/dist/shaka-player.compiled.js|9e8560c7265b520ab9d08e6bc2f946f7c22bebc3c6b77ec860541c670093cc0b
hls.js 1.7.4|hls.min.js|https://cdn.jsdelivr.net/npm/hls.js@1.7.4/dist/hls.min.js|cfc70cc9e9fc63d071919e087670abc760c8fd1bc889b88588026e4d22193bbe
"
# ----------------------------------------------------------------------------------------------

usage() {
    sed -n '/^# Fetch the third-party/,/^# Exit status/p' "$0" | sed 's/^# \{0,1\}//'
}

check_only=0
case "${1:-}" in
    "") ;;
    --check) check_only=1 ;;
    -h | --help) usage; exit 0 ;;
    *) printf 'fetch-mse-tools: unknown option: %s\n' "$1" >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || { printf 'fetch-mse-tools: too many arguments\n' >&2; exit 2; }

if [ -n "${MSE_TOOLS_DIR:-}" ]; then
    dir="$MSE_TOOLS_DIR"
elif [ "$(uname -s)" = Darwin ]; then
    dir="$HOME/Library/Caches/mercury-browser/ci-tools/mse"
else
    dir="${XDG_CACHE_HOME:-$HOME/.cache}/mercury-browser/ci-tools/mse"
fi

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

problems=0
problem() {
    printf 'fetch-mse-tools: %s\n' "$1" >&2
    problems=$((problems + 1))
}

# ---- players ---------------------------------------------------------------------------------
player_ok() { # file sha
    [ -f "$dir/players/$1" ] && [ "$(sha256_of "$dir/players/$1")" = "$2" ]
}

fetch_player() { # name file url sha
    if player_ok "$2" "$4"; then
        return 0
    fi
    [ "$check_only" -eq 0 ] || return 0
    command -v curl >/dev/null 2>&1 || { problem "curl not found"; return 0; }
    mkdir -p "$dir/players"
    part="$dir/players/$2.part"
    printf 'fetching %s\n' "$1" >&2
    if ! curl --fail --silent --show-error --location --max-time 300 --output "$part" "$3"; then
        rm -f "$part"
        return 0
    fi
    if [ "$(sha256_of "$part")" = "$4" ]; then
        mv "$part" "$dir/players/$2"
    else
        rm -f "$part"
        problem "$1: the download from $3 does not match the pinned SHA-256 $4 (refused and deleted)"
    fi
}

verify_player() { # name file url sha
    if [ ! -f "$dir/players/$2" ]; then
        problem "$1: $dir/players/$2 is missing"
    elif [ "$(sha256_of "$dir/players/$2")" != "$4" ]; then
        problem "$1: $dir/players/$2 has SHA-256 $(sha256_of "$dir/players/$2"), the pin is $4"
    fi
}

# ---- web-platform-tests ----------------------------------------------------------------------
wpt_head() {
    git -C "$dir/wpt" rev-parse HEAD 2>/dev/null || true
}

fetch_wpt() {
    [ "$(wpt_head)" = "$WPT_COMMIT" ] && return 0
    [ "$check_only" -eq 0 ] || return 0
    command -v git >/dev/null 2>&1 || { problem "git not found"; return 0; }
    printf 'fetching web-platform-tests %s (sparse: %s)\n' "$WPT_COMMIT" "$WPT_DIRS" >&2
    if [ ! -d "$dir/wpt/.git" ]; then
        mkdir -p "$dir/wpt"
        git -C "$dir/wpt" init --quiet
        git -C "$dir/wpt" remote add origin "$WPT_URL"
    fi
    # shellcheck disable=SC2086 # the directory list is split on purpose
    git -C "$dir/wpt" sparse-checkout set --cone $WPT_DIRS
    if git -C "$dir/wpt" fetch --quiet --depth 1 --filter=blob:none origin "$WPT_COMMIT" &&
        git -C "$dir/wpt" -c advice.detachedHead=false checkout --quiet --force FETCH_HEAD; then
        :
    else
        problem "could not fetch $WPT_COMMIT from $WPT_URL"
    fi
}

verify_wpt() {
    head="$(wpt_head)"
    if [ -z "$head" ]; then
        problem "$dir/wpt is missing (run without --check)"
        return 0
    fi
    if [ "$head" != "$WPT_COMMIT" ]; then
        problem "$dir/wpt is at $head, the pin is $WPT_COMMIT"
        return 0
    fi
    for d in $WPT_DIRS; do
        [ -d "$dir/wpt/$d" ] || problem "$dir/wpt/$d is missing"
    done
    [ -f "$dir/wpt/resources/testharness.js" ] || problem "$dir/wpt/resources/testharness.js is missing"
    [ -f "$dir/wpt/media-source/mediasource-util.js" ] || problem "$dir/wpt/media-source/mediasource-util.js is missing"
}

# ---- run -------------------------------------------------------------------------------------
fetch_wpt
verify_wpt
while IFS='|' read -r name file url sha; do
    [ -n "$name" ] || continue
    fetch_player "$name" "$file" "$url" "$sha"
    verify_player "$name" "$file" "$url" "$sha"
done <<EOF
$PLAYERS
EOF

if [ "$problems" -ne 0 ]; then
    printf 'fetch-mse-tools: %s problem(s); the tools in %s are NOT usable\n' "$problems" "$dir" >&2
    exit 1
fi

if [ "$check_only" -eq 0 ]; then
    {
        printf 'wpt %s %s\n' "$WPT_COMMIT" "$WPT_URL"
        while IFS='|' read -r name file url sha; do
            [ -n "$name" ] || continue
            printf '%s %s %s\n' "$name" "$file" "$sha"
        done <<EOF
$PLAYERS
EOF
    } >"$dir/VERSIONS"
fi
printf 'fetch-mse-tools: ok (%s): wpt %s, dash.js, Shaka Player, hls.js\n' "$dir" "$(printf '%s' "$WPT_COMMIT" | cut -c 1-10)"
