#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# The only way to build and run the GstMSE spike (lane sp-mse). The spike is a Cargo workspace of its
# own (spikes/mse/Cargo.toml); the repository's root workspace never includes it.
#
#   spikes/mse/run.sh <cargo-subcommand> [args...]
#
# Examples (always through the disk guard, in the light pool: the spike has no Servo in it):
#
#   MERCURY_SLOT_POOL=light ~/mercury-browser/.superpowers/tools/guarded.sh <worktree> \
#       'RUST_TEST_THREADS=2 spikes/mse/run.sh test -p mse-rig'
#   spikes/mse/run.sh clippy --all-targets -- -D warnings
#   spikes/mse/run.sh fmt -p mse-rig -- --check
#   spikes/mse/run.sh run --release -p mse-rig --example minimal
#   spikes/mse/run.sh run --release -p mse-conformance -- --report report.json
#
# What it does: changes to the repository root; loads the bundled GStreamer's build environment
# (scripts/macos/gstreamer.sh env: MERCURY_GSTREAMER_PREFIX, PKG_CONFIG*, PATH); sets
# DYLD_FALLBACK_LIBRARY_PATH to the framework's lib directory for cargo and so for the test and example
# binaries it starts (macOS strips DYLD_* variables when a SIP-protected shell starts, so the variable
# is set here, inside the script, and not inherited); adds --manifest-path spikes/mse/Cargo.toml and
# (where cargo takes it) --target-dir target, so that the build output is <worktree>/target, which
# the disk guard deletes; then execs cargo. `fmt` takes no target directory. Never build the root workspace.

set -eu

usage() {
    printf '%s\n' "usage: spikes/mse/run.sh <cargo-subcommand> [args...]   (for example: test -p mse-rig)" >&2
    exit 2
}

[ "$#" -ge 1 ] || usage
subcommand="$1"
shift

here="$(cd "$(dirname -- "$0")" && pwd -P)"
root="$(cd "$here/../.." && pwd -P)"
cd "$root"

# `gstreamer.sh env` explains the failure itself; the extra line names the make target.
if ! gst_env="$(scripts/macos/gstreamer.sh env)"; then
    printf '%s\n' "run.sh: the bundled GStreamer is not unpacked; run: make gstreamer-macos" >&2
    exit 1
fi
eval "$gst_env"

if [ -z "${MERCURY_GSTREAMER_PREFIX:-}" ] || [ ! -d "$MERCURY_GSTREAMER_PREFIX/lib" ]; then
    printf '%s\n' "run.sh: MERCURY_GSTREAMER_PREFIX is not a GStreamer prefix; run: make gstreamer-macos" >&2
    exit 1
fi
DYLD_FALLBACK_LIBRARY_PATH="$MERCURY_GSTREAMER_PREFIX/lib"
export DYLD_FALLBACK_LIBRARY_PATH

case "$subcommand" in
    build | b | check | c | clippy | test | t | run | r | doc | d | bench | rustc | rustdoc | fix | clean)
        exec cargo "$subcommand" --manifest-path spikes/mse/Cargo.toml --target-dir target "$@"
        ;;
    *)
        # fmt, tree, metadata, update, ...: no target directory.
        exec cargo "$subcommand" --manifest-path spikes/mse/Cargo.toml "$@"
        ;;
esac
