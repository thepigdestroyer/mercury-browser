#!/bin/bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Runs one shell command in its own session, so it survives an OpenCode server restart (which kills
# the server's background shells and their process groups). Output goes to <log>; the exit code is
# written to <log>.exit when the command ends, and the process id to <log>.pid.
#
# Usage: detach.sh <log> '<command>'      then: wait-for.sh <log>
set -euo pipefail
log="${1:?usage: detach.sh <log> <command>}"
shift
cmd="$*"
mkdir -p "$(dirname "$log")"
# Inside a build slot (the disk-guard plugin wrapped this shell command, or guarded.sh did), that slot
# is released as soon as this script returns. Tell a guarded.sh inside the job which slot process
# started it, so it can run in that slot only while it is still held and claim its own otherwise.
parent_slot=""
if [ -n "${MERCURY_IN_SLOT:-}${MERCURY_GUARDED_COMMAND:-}" ]; then
  state="${MERCURY_GUARD_STATE:-$HOME/mercury-browser/.superpowers/disk-guard}"
  owners=" $(for f in "$state"/slots/*/owner "$state"/light-slots/*/owner; do if [ -f "$f" ]; then head -n 1 "$f" || true; fi; done | tr '\n' ' ' || true) "
  p=$$
  while [ -n "$p" ] && [ "$p" -gt 1 ]; do
    case "$owners" in *" $p "*) parent_slot=$p; break ;; esac
    p="$(ps -o ppid= -p "$p" 2>/dev/null | tr -d ' ' || true)"
  done
fi
export MERCURY_DETACHED=1 MERCURY_PARENT_SLOT_PID="$parent_slot"
rm -f "$log.exit" "$log.pid"
nohup perl -MPOSIX -e 'POSIX::setsid(); exec @ARGV' /bin/bash -c "( $cmd ); echo \$? > '$log.exit'" > "$log" 2>&1 < /dev/null &
echo $! > "$log.pid"
echo "detached pid $(cat "$log.pid") -> $log (exit code in $log.exit)"
