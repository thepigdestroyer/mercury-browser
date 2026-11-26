#!/bin/bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Waits for a command started with detach.sh to end, then prints its exit code and the end of its log
# (without "Compiling" lines). Prints KILLED if the process is gone without writing an exit code.
# Safe to re-run after a server restart: it only watches files and a pid.
#
# Usage: wait-for.sh <log> [lines]   (run it with the shell tool's background option)
log="${1:?usage: wait-for.sh <log> [lines]}"
lines="${2:-40}"
pid="$(cat "$log.pid" 2>/dev/null || true)"
while [ ! -f "$log.exit" ]; do
    if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then
        sleep 2
        [ -f "$log.exit" ] && break
        echo "KILLED (pid $pid gone, no exit code)"
        grep -v '^ *Compiling' "$log" | tail -n "$lines"
        exit 99
    fi
    sleep 20
done
echo "EXIT $(cat "$log.exit")"
grep -v '^ *Compiling' "$log" | tail -n "$lines"
