#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Checks the build-slot tooling in scripts/dev without a build: the slot queue of build-slot.sh
# (priorities, ageing, dead waiters, the wait limit), and the detach, wait and guard scripts.
# Everything runs against a temporary state directory and the light pool with one or two slots, so it
# never touches the real slots, and it takes about a minute.
#
#   scripts/dev/test-build-tooling.sh      (or: make test-dev-scripts)
#   MERCURY_TEST_SECTIONS="queue" scripts/dev/test-build-tooling.sh    runs only those sections
#
# Prints one line per check and exits non-zero if any fails. Writes nothing except a temporary directory,
# which it removes.
set -uo pipefail

cd "$(dirname -- "${BASH_SOURCE[0]}")/../.." || exit 1
dev="$PWD/scripts/dev"
slot_script="${MERCURY_TEST_SLOT_SCRIPT:-$dev/build-slot.sh}"
sections="${MERCURY_TEST_SECTIONS:-queue detach}"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/mercury-dev-tests.XXXXXX")"
failures=0
counter=0

# Nothing from an enclosing build slot or a previous run may leak into the checks.
unset MERCURY_IN_SLOT MERCURY_GUARDED_COMMAND MERCURY_GUARDED_SHELL MERCURY_GUARDED_WORKTREE \
    MERCURY_DETACHED MERCURY_PARENT_SLOT_PID MERCURY_SLOT_PRIORITY MERCURY_BUILD_SLOTS \
    MERCURY_SLOT_WAIT_MINUTES MERCURY_SLOT_AGING_SECONDS CARGO_BUILD_JOBS
export MERCURY_SLOT_POOL=light MERCURY_LIGHT_SLOTS=1 MERCURY_LIGHT_MIN_FREE_GB=0 MERCURY_SLOT_POLL_SECONDS=1

# shellcheck disable=SC2329 # invoked by the EXIT trap below
cleanup() {
    local pids
    pids="$(jobs -p)"
    # shellcheck disable=SC2086
    [ -n "$pids" ] && kill $pids 2>/dev/null
    rm -rf "$tmp"
}
trap cleanup EXIT

check() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        printf 'PASS %s\n' "$name"
    else
        printf 'FAIL %s: expected [%s], got [%s]\n' "$name" "$expected" "$actual"
        failures=$((failures + 1))
    fi
}

wants() { case " $sections " in *" $1 "*) return 0 ;; esac; return 1; }

# A fresh state directory and order file for each check.
fresh() {
    counter=$((counter + 1))
    export MERCURY_GUARD_STATE="$tmp/state-$counter"
    order="$tmp/order-$counter"
    : >"$order"
}

# queue <label> <priority> <command>: runs <command> through build-slot.sh in the background; the
# label goes to the order file when the command starts, the exit status to $tmp/<label>.exit.
queue() {
    local label="$1" prio="$2" body="$3"
    (
        [ "$prio" != - ] && export MERCURY_SLOT_PRIORITY="$prio"
        MERCURY_GUARDED_COMMAND="echo $label >> '$order'; $body" MERCURY_GUARDED_SHELL=/bin/bash \
            MERCURY_GUARDED_WORKTREE="$tmp/wt-$label" bash "$slot_script" >>"$tmp/$label.out" 2>&1
        echo $? >"$tmp/$label.exit"
    ) &
}

queue_now() {
    queue "$@"
    wait "$!"
}

order_of() { tr '\n' ' ' <"$order" | sed 's/ $//'; }

if wants queue; then
    # A priority-2 waiter yields to a priority-1 waiter that arrived later.
    fresh
    queue holder 1 'sleep 4'
    sleep 1
    queue ahead 2 'sleep 1'
    sleep 1
    queue due 1 'sleep 1'
    wait
    check "queue: a priority-1 waiter goes before an earlier priority-2 waiter" "holder due ahead" "$(order_of)"

    # Ageing: every MERCURY_SLOT_AGING_SECONDS of waiting improves the priority by one, so the
    # earlier, lower-priority waiter gets through (and a tie goes to the longest waiter).
    fresh
    export MERCURY_SLOT_AGING_SECONDS=2
    queue holder 1 'sleep 9'
    sleep 1
    queue ahead 2 'sleep 1'
    sleep 3
    queue due 1 'sleep 1'
    wait
    unset MERCURY_SLOT_AGING_SECONDS
    check "queue: ageing lets a long-waiting priority-2 waiter through" "holder ahead due" "$(order_of)"

    # Priority 0 (the train integrator) goes before everything that waits.
    fresh
    queue holder 1 'sleep 4'
    sleep 1
    queue late 1 'sleep 1'
    sleep 1
    queue train 0 'sleep 1'
    wait
    check "queue: priority 0 goes before priority 1" "holder train late" "$(order_of)"

    # No priority set: first come, first served, as before.
    fresh
    queue holder - 'sleep 4'
    sleep 1
    queue first - 'sleep 1'
    sleep 1
    queue second - 'sleep 1'
    wait
    check "queue: without a priority it is first come, first served" "holder first second" "$(order_of)"

    # A nonsense priority counts as the default.
    fresh
    queue holder - 'sleep 3'
    sleep 1
    queue junk abc 'sleep 1'
    wait
    check "queue: a priority that is not a number counts as 1" "holder junk" "$(order_of)"

    # A waiter registers while it queues and is gone afterwards.
    fresh
    queue holder 1 'sleep 3'
    sleep 1
    queue waiting 2 'true'
    sleep 1
    files="$(find "$MERCURY_GUARD_STATE/waiting/light" -type f -name '[0-9]*' 2>/dev/null | wc -l | tr -d ' ')"
    check "queue: a queued build registers in waiting/light/<pid>" "1" "$files"
    registered="$(head -n 6 "$(find "$MERCURY_GUARD_STATE/waiting/light" -type f -name '[0-9]*' | head -n 1)" | sed -n '2p;4p;5p' | tr '\n' ' ')"
    check "queue: the registration names the worktree, the pool and the priority" "$tmp/wt-waiting light 2 " "$registered"
    wait
    files="$(find "$MERCURY_GUARD_STATE/waiting" -type f 2>/dev/null | wc -l | tr -d ' ')"
    check "queue: nothing stays registered once the builds end" "0" "$files"

    # A waiter whose process has gone is ignored, and its file removed.
    fresh
    mkdir -p "$MERCURY_GUARD_STATE/waiting/light"
    sleep 0 &
    dead=$!
    wait "$dead"
    printf '%s\nx\nnow\nlight\n0\n%s\n' "$dead" "$(date +%s)" >"$MERCURY_GUARD_STATE/waiting/light/$dead"
    queue after 1 'true'
    wait
    check "queue: a dead priority-0 waiter does not hold a build back" "after" "$(order_of)"
    check "queue: the dead waiter's registration is removed" "no" "$([ -e "$MERCURY_GUARD_STATE/waiting/light/$dead" ] && echo yes || echo no)"

    # A live better waiter keeps a free slot from a worse one; the wait limit still ends in exit 75.
    fresh
    mkdir -p "$MERCURY_GUARD_STATE/waiting/light"
    sleep 30 &
    live=$!
    printf '%s\nx\nnow\nlight\n0\n%s\n' "$live" "$(date +%s)" >"$MERCURY_GUARD_STATE/waiting/light/$live"
    MERCURY_SLOT_WAIT_MINUTES=0 queue_now blocked 1 'true'
    check "queue: a live better waiter blocks a free slot until the wait limit (exit 75)" "75" "$(cat "$tmp/blocked.exit" 2>/dev/null)"
    check "queue: ...and the blocked build never ran" "" "$(order_of)"
    kill "$live" 2>/dev/null
    wait "$live" 2>/dev/null

    # Another pool's waiters do not count.
    fresh
    mkdir -p "$MERCURY_GUARD_STATE/waiting/heavy"
    sleep 30 &
    other=$!
    printf '%s\nx\nnow\nheavy\n0\n%s\n' "$other" "$(date +%s)" >"$MERCURY_GUARD_STATE/waiting/heavy/$other"
    queue light 2 'true'
    sleep 3
    check "queue: a waiter of the other pool does not hold a build back" "light" "$(order_of)"
    kill "$other" 2>/dev/null
    wait "$other" 2>/dev/null

    # The rest of the old behaviour: exit status, the wait limit with a full pool, the job split.
    fresh
    queue status - 'exit 7'
    wait
    check "queue: the command's exit status is the script's" "7" "$(cat "$tmp/status.exit")"
    fresh
    queue holder - 'sleep 4'
    sleep 1
    MERCURY_SLOT_WAIT_MINUTES=0 queue gaveup - 'true'
    sleep 2
    check "queue: exit 75 after the wait limit (a tiny limit) when no slot is free" "75" "$(cat "$tmp/gaveup.exit" 2>/dev/null)"
    wait
    check "queue: ...and it did not run" "holder" "$(order_of)"
    fresh
    # shellcheck disable=SC2016 # expanded by the command's own shell, inside the slot
    queue env - 'echo "in=$MERCURY_IN_SLOT jobs=$CARGO_BUILD_JOBS" > '"'$tmp/env.txt'"
    wait
    case "$(cat "$tmp/env.txt")" in
        "in=1 jobs="[0-9]*) check "queue: the command runs with MERCURY_IN_SLOT=1 and CARGO_BUILD_JOBS set" ok ok ;;
        *) check "queue: the command runs with MERCURY_IN_SLOT=1 and CARGO_BUILD_JOBS set" ok "$(cat "$tmp/env.txt")" ;;
    esac
fi

if wants detach; then
    export MERCURY_PARENT_SLOT_GRACE_SECONDS=1 MERCURY_WAIT_POLL_SECONDS=1
    guarded="$dev/guarded.sh"
    detach="$dev/detach.sh"
    waitfor="$dev/wait-for.sh"
    work="$tmp/work"
    mkdir -p "$work"
    # A job that reports how many light slots are held and whether it runs inside one.
    count="$work/count.sh"
    # shellcheck disable=SC2016 # the script's own shell expands these
    printf '#!/bin/bash\nn=$(ls "$MERCURY_GUARD_STATE/light-slots" 2>/dev/null | wc -l | tr -d " ")\necho "slots=$n in=${MERCURY_IN_SLOT:-no}"\n' >"$count"
    chmod +x "$count"

    # (a) detach only: the detached job's own guarded.sh claims a slot of its own.
    fresh
    log="$tmp/a.log"
    (cd "$work" && MERCURY_IN_SLOT=1 MERCURY_GUARDED_COMMAND=x "$detach" "$log" "$guarded $work '$count'" >/dev/null)
    "$waitfor" "$log" >"$tmp/a.wait"
    check "(a) detach only: the job claims its own slot" "slots=1 in=1" "$(grep '^slots=' "$log")"
    check "(a) detach only: wait-for reports the exit code" "EXIT 0" "$(head -n 1 "$tmp/a.wait")"

    # (b) detach then wait in one slot: the job reuses the outer slot, so exactly one is held.
    fresh
    log="$tmp/b.log"
    queue_now b-outer - "cd '$work' && '$detach' '$log' \"$guarded $work '$count'\" >/dev/null && '$waitfor' '$log' >'$tmp/b.wait'"
    check "(b) detach then wait: the job reuses the outer slot" "slots=1 in=1" "$(grep '^slots=' "$log")"
    check "(b) detach then wait: wait-for returns the job's exit code" "EXIT 0" "$(head -n 1 "$tmp/b.wait")"

    # (c) nesting: a guarded.sh inside a command that already holds a slot holds no second slot.
    fresh
    log="$tmp/c1.log"
    queue_now c-outer - "$guarded $work '$count' >'$log'"
    check "(c) nested guarded.sh holds one slot" "slots=1 in=1" "$(cat "$log")"
    fresh
    log="$tmp/c2.log"
    queue_now c-outer - "$guarded $work \"$guarded $work '$count'\" >'$log'"
    check "(c) guarded.sh inside guarded.sh holds one slot" "slots=1 in=1" "$(cat "$log")"

    # (d) build-slot.sh with a tiny wait limit gives up with exit 75 while every slot is taken.
    fresh
    queue d-holder - 'sleep 4'
    sleep 1
    MERCURY_SLOT_WAIT_MINUTES=0 queue d-tiny - 'true'
    wait
    check "(d) build-slot.sh with a tiny wait limit exits 75" "75" "$(cat "$tmp/d-tiny.exit" 2>/dev/null)"
fi

if [ "$failures" -ne 0 ]; then
    printf '%s check(s) failed\n' "$failures" >&2
    exit 1
fi
printf '%s\n' 'build tooling: all checks passed'
