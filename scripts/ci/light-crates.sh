#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Print the cargo `-p` arguments for every workspace member that can be built
# without the browser engine or the UI toolkit: members whose full dependency
# closure (normal, build and dev dependencies, for every target platform)
# contains neither the `servo` crate nor `slint`.
#
#   scripts/ci/light-crates.sh            # -p talaria-core -p …   (one line)
#   scripts/ci/light-crates.sh --explain  # one line per member, light or not, and why
#
# The list is derived from `cargo metadata --locked` every time, so a new
# library crate is picked up without editing anything here. The graph is read
# by `light-crates` (Rust, crates/mercury-devtools), which this builds in
# release mode the first time; nothing else is compiled. Cargo's messages go
# to standard error. Exits non-zero if no member qualifies, since a fast job
# with nothing to check would pass for the wrong reason.
set -euo pipefail

usage() {
    sed -n '/^# Print the cargo/,/^# with nothing to check/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

EXPLAIN=()
case "${1:-}" in
    "") ;;
    --explain) EXPLAIN=(--explain) ;;
    -h | --help) usage; exit 0 ;;
    *) printf 'light-crates: unknown option: %s\n' "$1" >&2; exit 2 ;;
esac

cd "$(dirname -- "${BASH_SOURCE[0]}")/../.."

command -v "${CARGO:-cargo}" >/dev/null 2>&1 || { printf 'light-crates: cargo not found\n' >&2; exit 1; }

# The metadata (several megabytes, too large for the environment or the
# command line) goes through a temporary file to standard input, so that a
# cargo failure stops here with cargo's own message.
METADATA_FILE="$(mktemp)"
trap 'rm -f -- "$METADATA_FILE"' EXIT
"${CARGO:-cargo}" metadata --locked --format-version 1 >"$METADATA_FILE"
"${CARGO:-cargo}" run --locked --release --quiet --package mercury-devtools --bin light-crates -- \
    ${EXPLAIN[@]+"${EXPLAIN[@]}"} <"$METADATA_FILE"
