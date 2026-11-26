#!/bin/bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Claude Code stand-in for .opencode/plugins/mercury-disk-guard.ts: gives a build command the same
# environment the plugin sets and runs it through scripts/dev/build-slot.sh.
# Usage: guarded.sh <worktree> <command...>   (MERCURY_SLOT_POOL=light for engine-free -p builds)
#
# Nesting: when this runs inside a build slot already (the disk-guard plugin wrapped the calling
# shell command, or an outer guarded.sh did), it runs the command directly in the slot it is in.
# Claiming a second slot from inside the first deadlocked the light pool on 2026-11-22: every slot
# was held by an outer command whose inner guarded.sh waited for a slot.
set -euo pipefail
wt="$(cd "${1:?worktree}" && pwd)"; shift
here="$(cd "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_RELEASE_DEBUG=0
export CARGO_FLAGS="${CARGO_FLAGS:---release}"
export CI_TOOLS_DIR="$HOME/Library/Caches/mercury-browser/ci-tools"
# Shared compiler cache across lane worktrees (sccache in ~/.local/bin, verified release).
# Outside ~/Library/Caches/mercury-browser: the repo's disk-guard plugin deletes every entry there except
# gstreamer and ci-tools whenever no mercury-browser runs (it wiped the cache on 2026-11-22). Own port, so
# builds still talking to the old server on the default port are not cut off.
if command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER=sccache SCCACHE_DIR="$HOME/Library/Caches/mercury-sccache" SCCACHE_SERVER_PORT=4227 SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-60G}" SCCACHE_IDLE_TIMEOUT=3600
fi
# Detached jobs (detach.sh, 2026-11-22): the slot that launched detach.sh is released the moment
# detach.sh returns, unless the same shell command goes on waiting (detach.sh ...; wait-for.sh ...).
# detach.sh records that slot's process in MERCURY_PARENT_SLOT_PID; give it a moment to exit, and run
# in it only while it is still held. Otherwise claim a slot of our own, so detached builds never run
# outside the pool (they did: the plugin's env made them look nested).
parent_slot_held() {
  sleep 3
  [ -n "${MERCURY_PARENT_SLOT_PID:-}" ] && kill -0 "$MERCURY_PARENT_SLOT_PID" 2>/dev/null
}
if [ -n "${MERCURY_IN_SLOT:-}" ] || [ -n "${MERCURY_GUARDED_COMMAND:-}" ]; then
  if [ -z "${MERCURY_DETACHED:-}" ] || parent_slot_held; then
    unset MERCURY_DETACHED MERCURY_PARENT_SLOT_PID
    cd "$wt"
    exec /bin/zsh -c "$*"
  fi
  unset MERCURY_IN_SLOT MERCURY_GUARDED_COMMAND MERCURY_GUARDED_SHELL MERCURY_GUARDED_WORKTREE
fi
unset MERCURY_DETACHED MERCURY_PARENT_SLOT_PID
export MERCURY_GUARD_STATE="${MERCURY_GUARD_STATE:-$HOME/mercury-browser/.superpowers/disk-guard}"
export MERCURY_GUARDED_COMMAND="cd '$wt' && $*"
export MERCURY_GUARDED_SHELL=/bin/zsh
export MERCURY_GUARDED_WORKTREE="$wt"
export MERCURY_IN_SLOT=1
mkdir -p "$MERCURY_GUARD_STATE"
exec /bin/bash "$here/build-slot.sh"
