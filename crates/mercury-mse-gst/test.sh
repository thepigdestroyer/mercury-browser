#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# Runs cargo for the GStreamer half of the MSE backend (lane med-1, item 4) with the bundled GStreamer's
# build environment, as the root Makefile and the sp-mse spike's run.sh do. Always through the disk guard,
# in the light pool (the crate has no Servo in it), with a target directory of its own inside the
# worktree's target/ so that cargo's directory lock is not shared with a Servo build:
#
#   MERCURY_SLOT_POOL=light ~/mercury-browser/.superpowers/tools/guarded.sh <worktree> \
#       'RUST_TEST_THREADS=2 crates/mercury-mse-gst/test.sh test --release -p mercury-mse-gst'
#   crates/mercury-mse-gst/test.sh clippy --release -p mercury-mse-gst --all-targets -- -D warnings
#
# What it does: changes to the repository root; loads scripts/macos/gstreamer.sh env (the framework's
# PKG_CONFIG paths and PATH); sets DYLD_FALLBACK_LIBRARY_PATH to the framework's lib directory on the
# cargo command itself (macOS strips DYLD_* variables when a SIP-protected shell starts); defaults
# CARGO_TARGET_DIR to <worktree>/target/light; then execs cargo with your arguments.

set -eu

[ "$#" -ge 1 ] || {
    printf '%s\n' "usage: crates/mercury-mse-gst/test.sh <cargo-subcommand> [args...]" >&2
    exit 2
}

here="$(cd "$(dirname -- "$0")" && pwd -P)"
root="$(cd "$here/../.." && pwd -P)"
cd "$root"

if ! gst_env="$(scripts/macos/gstreamer.sh env)"; then
    printf '%s\n' "test.sh: the bundled GStreamer is not unpacked; run: make gstreamer-macos" >&2
    exit 1
fi
eval "$gst_env"

if [ -z "${MERCURY_GSTREAMER_PREFIX:-}" ] || [ ! -d "$MERCURY_GSTREAMER_PREFIX/lib" ]; then
    printf '%s\n' "test.sh: MERCURY_GSTREAMER_PREFIX is not a GStreamer prefix; run: make gstreamer-macos" >&2
    exit 1
fi
DYLD_FALLBACK_LIBRARY_PATH="$MERCURY_GSTREAMER_PREFIX/lib"
export DYLD_FALLBACK_LIBRARY_PATH
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/light}"
export CARGO_TARGET_DIR

exec cargo "$@"
