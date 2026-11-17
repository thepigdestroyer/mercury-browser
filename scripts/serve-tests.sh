#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# Serve the repository's tests/ directory on the loopback interface, for the media and dialog
# checks.
#
#   scripts/serve-tests.sh [--port N]
#
# Builds (quietly, if needed) and runs the Rust server `serve-tests` from
# crates/mercury-devtools, in place of this shell: it binds 127.0.0.1 on an ephemeral port (or
# --port), prints the port on the first line of standard output, and serves tests/ (so
# tests/pages/media.html is /pages/media.html). Single byte ranges are answered with 206
# Partial Content; /private asks for HTTP Basic authentication (realm "Mercury test realm",
# only "alice" / "secret"). It exits on SIGINT, SIGTERM or SIGHUP, and when the process that
# started it goes away. Cargo's messages go to standard error.
set -eu

repository="$(cd "$(dirname -- "$0")/.." && pwd -P)"
"${CARGO:-cargo}" build --locked --release --quiet \
    --manifest-path "$repository/Cargo.toml" -p mercury-devtools --bin serve-tests >&2
case "${CARGO_TARGET_DIR:-}" in
    '') target="$repository/target" ;;
    /*) target="$CARGO_TARGET_DIR" ;;
    *) target="$PWD/$CARGO_TARGET_DIR" ;;
esac
exec "$target/release/serve-tests" "$@"
