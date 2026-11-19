#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# Serve the repository's tests/ directory on the loopback interface, for the media and dialog
# checks.
#
#   scripts/serve-tests.sh [-h] [--port N] [--log FILE]
#
# Builds (quietly, if needed) and runs the Rust server `serve-tests` from
# crates/mercury-devtools, in place of this shell: it binds 127.0.0.1 on an ephemeral port (or
# --port), prints the port on the first line of standard output, and serves tests/ (so
# tests/pages/media.html is /pages/media.html). Single byte ranges are answered with 206
# Partial Content; /private asks for HTTP Basic authentication (realm "Mercury test realm",
# only "alice" / "secret"). It exits on SIGINT, SIGTERM or SIGHUP, and when the process that
# started it goes away. Cargo's messages go to standard error.
#
# `cargo run` replaces itself with the server, as this shell replaces itself with cargo, so
# the process started in the background ($!) is the server and its parent is the caller.
# Cargo finds the executable wherever the target directory is configured to be. Without cargo
# the script prints "serve-tests: cargo not found" to standard error and exits with status 1.
set -eu

repository="$(cd "$(dirname -- "$0")/.." && pwd -P)"
# A shell built-in, so that the exec below is still what starts the server.
command -v "${CARGO:-cargo}" >/dev/null 2>&1 || { printf 'serve-tests: cargo not found\n' >&2; exit 1; }
exec "${CARGO:-cargo}" run --locked --release --quiet \
    --manifest-path "$repository/Cargo.toml" -p mercury-devtools --bin serve-tests -- "$@"
