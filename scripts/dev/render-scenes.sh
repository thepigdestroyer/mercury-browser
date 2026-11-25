#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Renders every picture that mercury-ui's examples and render tests can write, with Slint's
# software renderer (no window, no GPU), into <output-dir>: one folder for each example
# (`example-<name>/`) and each test (`test-<name>/`). Run it on two commits and compare the two
# folders with `pixel-diff` (crates/mercury-devtools): that is how the theme-plumbing lane proved
# that the new theme layer draws exactly what the chrome drew before.
#
# Usage: scripts/dev/render-scenes.sh <output-dir>
# Build the way the repository's other builds do (see the mercury-build-hygiene skill): through the
# disk guard / `.superpowers/tools/guarded.sh <worktree> scripts/dev/render-scenes.sh <dir>`, with
# MERCURY_SLOT_POOL=light (mercury-ui needs no engine).
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <output-dir>" >&2
    exit 2
fi
out="$1"
cd "$(dirname "$0")/../.."

mkdir -p "$out"
for source in crates/mercury-ui/examples/render_*.rs; do
    name="$(basename "$source" .rs)"
    mkdir -p "$out/example-$name"
    cargo run --release -p mercury-ui --features test-support --example "$name" -- "$out/example-$name"
done

# Every test file that honours MERCURY_UI_RENDER_DIR, so a test added later is included.
for source in crates/mercury-ui/tests/*.rs; do
    grep -q MERCURY_UI_RENDER_DIR "$source" || continue
    name="$(basename "$source" .rs)"
    mkdir -p "$out/test-$name"
    MERCURY_UI_RENDER_DIR="$out/test-$name" \
        cargo test --release -p mercury-ui --features test-support --test "$name" -- --test-threads=1
done
