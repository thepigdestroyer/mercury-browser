#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# The window run of the Media Source Extensions work, in one command (docs/developer-guide/mse-testing.md):
#
#   scripts/dev/mse-window-run.sh pages|players|wpt|all [--resume] [--timeout SECONDS] [--only TEXT] [--list]
#                                 [--browser PATH] [--no-lock-check] ...
#   scripts/dev/mse-window-run.sh verdict [--gates-green] [--review-ok] [--allow-engine-gap NAME] ...

# `verdict` decides the MSE flip from the journal (F1-F5; F6-F7 need the flags), writes flip-verdict-<UTC>.md
# and exits 0 GO, 1 NO-GO or 3 INCOMPLETE. It runs no browser and needs no tools.
#
# Everything after the phase goes to `mse-run` (crates/mercury-devtools; see `mse-run --help`), which
# refuses to start when the screen is locked, starts the test server, runs each page or test in its own
# browser process with a watchdog, writes a journal line after every test (a lock in the middle of a run
# costs one test: --resume continues) and writes the report window-run-<UTC>.md.
#
# This script only sets the environment the browser needs: the tools fetched (players and wpt phases:
# scripts/dev/fetch-mse-tools.sh --check), the GStreamer the build uses (scripts/macos/gstreamer.sh env),
# the libraries next to target/release/mercury-browser, and the output directory (MSE_RUN_OUT; by default
# the lane's log directory when it exists, else a temporary one). Build the browser first with
# `make build-release`; the browser is started with MERCURY_DEV_MSE=1 by mse-run.
#
# Exit status: that of mse-run (0 clean, 1 unexpected results, 2 usage or preflight, 3 incomplete).
set -eu

repository="$(cd "$(dirname -- "$0")/.." && cd .. && pwd -P)"
cd "$repository"

case "${1:-}" in
    -h | --help)
        sed -n '/^# The window run/,/^# Exit status/p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    pages | players | wpt | all | verdict | "") ;;
    *) ;;
esac

# The tools are needed by the players and wpt phases (and by `all`).
case "${1:-}" in
    players | wpt | all)
        if ! scripts/dev/fetch-mse-tools.sh --check; then
            printf 'mse-window-run: run scripts/dev/fetch-mse-tools.sh first\n' >&2
            exit 2
        fi
        ;;
    *) ;;
esac

if [ -z "${MSE_RUN_OUT:-}" ]; then
    if [ -d "$HOME/mercury-browser/.superpowers/sdd/med-1b-logs" ]; then
        MSE_RUN_OUT="$HOME/mercury-browser/.superpowers/sdd/med-1b-logs"
    else
        MSE_RUN_OUT="${TMPDIR:-/tmp}/mercury-mse-run"
    fi
    export MSE_RUN_OUT
fi

# The GStreamer of the build, as `make` sets it up. Not needed for a stub browser or --list.
if [ "$(uname -s)" = Darwin ] && [ -x scripts/macos/gstreamer.sh ]; then
    if gst_env="$(scripts/macos/gstreamer.sh env 2>/dev/null)"; then
        eval "$gst_env"
    else
        printf 'mse-window-run: no GStreamer (make gstreamer-macos); continuing, a stub browser does not need it\n' >&2
    fi
fi
if [ -d "$repository/target/release/lib" ]; then
    DYLD_FALLBACK_LIBRARY_PATH="$repository/target/release/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
    export DYLD_FALLBACK_LIBRARY_PATH
fi

command -v "${CARGO:-cargo}" >/dev/null 2>&1 || { printf 'mse-window-run: cargo not found\n' >&2; exit 1; }
exec "${CARGO:-cargo}" run --locked --release --quiet \
    --manifest-path "$repository/Cargo.toml" -p mercury-devtools --bin mse-run -- "$@"
