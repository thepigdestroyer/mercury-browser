#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# Runs the Servo tests of crates/mercury-engine-tests (`make engine-tests`), each in a process of
# its own. Servo keeps its options in a process-wide `OnceLock`, so only one `Servo` can be built
# per process: a second test in the same process panics with "Already initialized" (Servo's own CI
# runs one test per process too). Needs MERCURY_GSTREAMER_PREFIX (scripts/macos/gstreamer.sh env on
# macOS; empty elsewhere). CARGO_FLAGS (for example --release) is passed to cargo as it is.
set -eu

cargo="${CARGO:-cargo}"
prefix="${MERCURY_GSTREAMER_PREFIX:-}"
failed=0
passed=0

run() {
    # DYLD_* variables do not survive a SIP-protected shell, so they are set on the command line.
    # shellcheck disable=SC2086
    DYLD_FALLBACK_LIBRARY_PATH="${prefix:+$prefix/lib}" "$cargo" test --locked ${CARGO_FLAGS:-} \
        -p mercury-engine-tests "$@"
}

for target in input_methods mercury_ime; do
    tests="$(run --test "$target" -- --list --format terse 2>/dev/null | sed -n 's/: test$//p')"
    if [ -z "$tests" ]; then
        echo "FAILED: no tests listed for $target" >&2
        exit 1
    fi
    for name in $tests; do
        if log="$(run --test "$target" -- --exact "$name" --test-threads=1 2>&1)"; then
            echo "ok     $target::$name"
            passed=$((passed + 1))
        else
            echo "FAILED $target::$name"
            printf '%s\n' "$log" | tail -40
            failed=$((failed + 1))
        fi
    done
done

echo "engine tests: $passed passed, $failed failed"
[ "$failed" -eq 0 ]
