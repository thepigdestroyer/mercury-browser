#!/bin/bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Runs one shell command while holding one of Mercury's build slots, so that only a few builds run
# on the development Mac at once and none starts without enough free disk space.
#
# The OpenCode plugin .opencode/plugins/mercury-disk-guard.ts sends every build command here
# (cargo build/test/clippy/check/run, make lint/test/build-release/...). The command arrives in
# MERCURY_GUARDED_COMMAND and runs unchanged under MERCURY_GUARDED_SHELL; its exit status is ours.
# See .opencode/skills/mercury-build-hygiene/SKILL.md.
#
# Queue order (MERCURY_SLOT_PRIORITY): 0 = the train integrator and the hands-on window run,
# 1 = the default (lanes due in the next train), 2 = lanes writing code ahead for later waves.
# Every waiter registers in <state>/waiting/<pool>/<pid> (pid, worktrees, time, pool, priority,
# start time in epoch seconds) and takes a free slot only when no other live waiter of the same pool
# ranks ahead of it: a better effective priority, or the same one and a longer wait. Every
# MERCURY_SLOT_AGING_SECONDS (3600) of waiting improves the effective priority by one, so nothing
# starves. A waiter whose process has gone is ignored and removed. The plugin also counts a
# queued build's worktree as in use, so a build that waits never loses its target/ to the idle sweep.
#
# The command runs with MERCURY_IN_SLOT=1, which tells scripts/dev/guarded.sh (and anything else
# that would claim a slot) that it is already in one.
#
# Everything runs inside main(), and the file is only ever replaced (written elsewhere, then moved
# over), never edited in place: bash reads a running script lazily, so an in-place edit would break
# every build already waiting in it.
main() {
set -uo pipefail

state_dir="${MERCURY_GUARD_STATE:?MERCURY_GUARD_STATE is not set}"
command="${MERCURY_GUARDED_COMMAND:?MERCURY_GUARDED_COMMAND is not set}"
run_shell="${MERCURY_GUARDED_SHELL:-/bin/zsh}"
worktrees="${MERCURY_GUARDED_WORKTREE:-}"
# Two pools: "heavy" builds compile Servo; "light" builds touch only crates that don't depend on it
# (minutes, a few hundred MB), so they never queue behind a heavy one.
pool="${MERCURY_SLOT_POOL:-heavy}"
if [ "$pool" = light ]; then
    slots="${MERCURY_LIGHT_SLOTS:-4}"
    min_free_gb="${MERCURY_LIGHT_MIN_FREE_GB:-10}"
    slot_root="$state_dir/light-slots"
else
    pool=heavy
    slots="${MERCURY_BUILD_SLOTS:-6}"
    min_free_gb="${MERCURY_MIN_FREE_GB:-40}"
    slot_root="$state_dir/slots"
fi
wait_minutes="${MERCURY_SLOT_WAIT_MINUTES:-360}"
poll_seconds="${MERCURY_SLOT_POLL_SECONDS:-15}"
aging_seconds="${MERCURY_SLOT_AGING_SECONDS:-3600}"
case "$aging_seconds" in '' | *[!0-9]* | 0) aging_seconds=3600 ;; esac
priority="${MERCURY_SLOT_PRIORITY:-1}"
case "$priority" in '' | *[!0-9]*) priority=1 ;; esac
waiting_root="$state_dir/waiting/$pool"
waiter="$waiting_root/$$"
mkdir -p "$slot_root" "$waiting_root"

say() { printf 'mercury-disk-guard: %s\n' "$*" >&2; }

free_gb() { df -Pk "$HOME" | awk 'NR == 2 { print int($4 / 1048576) }'; }

# A slot is a directory: mkdir is atomic, so two builds can never take the same one. Its owner file
# names the process and the worktrees; a slot whose process has gone (killed, crashed) is reclaimed.
# A process that has exited but was never reaped by its parent (a zombie, state Z) still answers
# kill -0, yet holds nothing; it must not keep a slot.
process_alive() {
    kill -0 "$1" 2>/dev/null || return 1
    case "$(ps -o stat= -p "$1" 2>/dev/null)" in Z*) return 1 ;; esac
}

reclaim_dead_slots() {
    local dir pid age
    for dir in "$slot_root"/*/; do
        [ -d "$dir" ] || continue
        if [ -f "$dir/owner" ]; then
            pid="$(head -n 1 "$dir/owner")"
            if [ -n "$pid" ] && ! process_alive "$pid"; then
                rm -rf "$dir"
            fi
        else
            age=$(($(date +%s) - $(stat -f %m "$dir")))
            [ "$age" -gt 120 ] && rm -rf "$dir"
        fi
    done
}

holders() {
    local dir
    for dir in "$slot_root"/*/; do
        [ -f "$dir/owner" ] && sed -n 2p "$dir/owner"
    done | tr '\n' ' '
}

claim() {
    local i dir
    for i in $(seq 1 "$slots"); do
        dir="$slot_root/$i"
        if mkdir "$dir" 2>/dev/null; then
            printf '%s\n%s\n%s\n' "$$" "${worktrees:-?}" "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" >"$dir/owner"
            printf '%s\n' "$dir"
            return 0
        fi
    done
    return 1
}

# Written whole and moved into place, so nobody reads half a file.
register() {
    printf '%s\n%s\n%s\n%s\n%s\n%s\n' "$$" "${worktrees:-?}" "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$pool" \
        "$priority" "$started" >"$waiter.tmp" && mv "$waiter.tmp" "$waiter"
}

# Succeeds when another live waiter of this pool ranks ahead of this one: a lower effective
# priority (the priority minus one per aging period waited), or the same one and an earlier start
# (then the lower pid). Removes the registrations of waiters whose process has gone.
waiter_ahead() {
    local file pid pr start now eff mine
    now=$(date +%s)
    mine=$((priority - (now - started) / aging_seconds))
    for file in "$waiting_root"/*; do
        [ -f "$file" ] || continue
        # Lines: pid, worktrees, time, pool (not needed here), priority, start (epoch seconds).
        { read -r pid; read -r _; read -r _; read -r _; read -r pr; read -r start; } <"$file" || true
        case "$pid$pr$start" in '' | *[!0-9]*) continue ;; esac
        [ "$pid" = "$$" ] && continue
        if ! process_alive "$pid"; then
            rm -f "$file"
            continue
        fi
        eff=$((pr - (now - start) / aging_seconds))
        if [ "$eff" -lt "$mine" ]; then return 0; fi
        if [ "$eff" -eq "$mine" ]; then
            if [ "$start" -lt "$started" ]; then return 0; fi
            if [ "$start" -eq "$started" ] && [ "$pid" -lt "$$" ]; then return 0; fi
        fi
    done
    return 1
}

started=$(date +%s)
register
trap 'rm -f "$waiter" "$waiter.tmp"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

last_note=0
slot=""
while :; do
    reclaim_dead_slots
    [ -f "$waiter" ] || register
    free="$(free_gb)"
    if [ "$free" -ge "$min_free_gb" ] && ! waiter_ahead && slot="$(claim)"; then
        break
    fi
    now=$(date +%s)
    if [ $((now - started)) -ge $((wait_minutes * 60)) ]; then
        say "gave up after ${wait_minutes} min: free ${free} GB (need ${min_free_gb}), builds running in: $(holders)"
        exit 75
    fi
    if [ $((now - last_note)) -ge 300 ]; then
        if [ "$free" -lt "$min_free_gb" ]; then
            say "waiting: only ${free} GB free (a build needs ${min_free_gb} GB); old build output is being removed"
        else
            say "waiting for a build slot, priority ${priority} (${slots} in use: $(holders))"
        fi
        last_note=$now
    fi
    sleep "$poll_seconds"
done
rm -f "$waiter"

    # shellcheck disable=SC2329 # invoked by the EXIT trap below
release() {
    local wt
    rm -rf "$slot"
    rm -f "$waiter" "$waiter.tmp"
    # Record when each worktree last finished a build: the plugin removes build output that has
    # not been used for a while.
    for wt in $worktrees; do
        [ -d "$wt/target" ] && touch "$wt/target/.mercury-last-build"
    done
}
trap release EXIT

# Share the cores evenly between all the slots (at least two jobs each), not just the builds running
# now: a build that starts alone would otherwise keep every core for its whole run. A command that sets
# CARGO_BUILD_JOBS itself still wins, since it runs after this.
if [ -z "${CARGO_BUILD_JOBS:-}" ]; then
    cores="$(sysctl -n hw.ncpu 2>/dev/null || echo 8)"
    running="$slots"
    jobs=$((cores / (running > 0 ? running : 1)))
    export CARGO_BUILD_JOBS=$((jobs > 2 ? jobs : 2))
fi

# Tell everything below that it already holds a slot (scripts/dev/guarded.sh runs in it directly).
export MERCURY_IN_SLOT=1

"$run_shell" -c "$command"
}

main "$@"
exit $?
