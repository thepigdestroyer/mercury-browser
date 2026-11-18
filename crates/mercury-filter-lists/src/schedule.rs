// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! When the next automatic update check is due, as a pure function of times.
//!
//! Automatic checks run at most once every [`UPDATE_INTERVAL`] (24 hours) after a check that
//! reached the publishers, counted from its start, and never before [`SETTLE_DELAY`] after
//! start-up. After a check that could not reach them (no network, a time-out, a server error),
//! nothing was downloaded, so the next one comes sooner: 1 hour after the first such failure in
//! a row, then 2, 4, 8 and 16 hours, and 24 hours from the sixth on ([`retry_interval`]). A check is also due when the last one is recorded in the future (the clock was
//! set back), so a wrong clock cannot hold updates off for long. "Update now" ignores this.

use std::time::Duration;

/// The shortest time between two automatic checks.
pub const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How long after start-up the first automatic check waits, so it never competes with the
/// first pages.
pub const SETTLE_DELAY: Duration = Duration::from_secs(60);
/// The longest the updater sleeps before looking at the clock again, so that a computer that
/// slept through the due time checks soon after it wakes.
pub const RECHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// The time between the start of the last check and the next one, after `failures` checks in a
/// row that could not reach the publishers (0: the last one reached them).
pub fn retry_interval(failures: u32) -> Duration {
    match failures {
        0 => UPDATE_INTERVAL,
        failures => {
            let hours = 1_u64 << (failures - 1).min(5);
            Duration::from_secs(hours * 60 * 60).min(UPDATE_INTERVAL)
        }
    }
}

/// How long to wait before the next automatic check, from `now` (seconds since the Unix
/// epoch). `last_check` is when the last check started and `failures` how many checks in a row
/// could not reach the publishers; `since_start` is how long this run has been going. `Duration::ZERO` means a check is due now. The result is never longer than
/// [`RECHECK_INTERVAL`]: the caller asks again after waiting.
pub fn wait_before_check(
    last_check: Option<u64>,
    failures: u32,
    now: u64,
    since_start: Duration,
    settle_delay: Duration,
) -> Duration {
    let settling = settle_delay.saturating_sub(since_start);
    let until_due = match last_check {
        Some(last) if last <= now => {
            retry_interval(failures).saturating_sub(Duration::from_secs(now - last))
        }
        // Never checked, or the clock moved back.
        _ => Duration::ZERO,
    };
    settling.max(until_due).min(RECHECK_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_794_775_611;
    const DAY: u64 = 24 * 60 * 60;
    const SETTLED: Duration = Duration::from_secs(600);

    fn wait(last_check: Option<u64>, now: u64, since_start: Duration) -> Duration {
        wait_before_check(last_check, 0, now, since_start, SETTLE_DELAY)
    }

    #[test]
    fn never_checked_is_due_once_start_up_has_settled() {
        assert_eq!(wait(None, NOW, SETTLED), Duration::ZERO);
        assert_eq!(
            wait(None, NOW, Duration::from_secs(15)),
            Duration::from_secs(45)
        );
    }

    #[test]
    fn a_check_is_due_a_day_after_the_last_one() {
        assert_eq!(wait(Some(NOW - DAY), NOW, SETTLED), Duration::ZERO);
        assert_eq!(wait(Some(NOW - 3 * DAY), NOW, SETTLED), Duration::ZERO);
        assert_eq!(
            wait(Some(NOW - DAY + 120), NOW, SETTLED),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn waits_are_capped_so_the_clock_is_read_again() {
        assert_eq!(wait(Some(NOW - 60), NOW, SETTLED), RECHECK_INTERVAL);
        assert_eq!(wait(Some(NOW), NOW, SETTLED), RECHECK_INTERVAL);
    }

    #[test]
    fn the_settle_delay_holds_a_due_check_back() {
        assert_eq!(
            wait(Some(NOW - 2 * DAY), NOW, Duration::from_secs(59)),
            Duration::from_secs(1)
        );
        assert_eq!(
            wait_before_check(Some(NOW - 2 * DAY), 0, NOW, Duration::ZERO, Duration::ZERO),
            Duration::ZERO
        );
    }

    #[test]
    fn a_last_check_in_the_future_means_the_clock_moved_back_and_is_due() {
        assert_eq!(wait(Some(NOW + 3600), NOW, SETTLED), Duration::ZERO);
    }

    #[test]
    fn at_most_one_automatic_check_per_day_whatever_the_times() {
        // Simulate a week of a browser that is always running, waking whenever told to.
        let mut now = NOW;
        let mut last_check = None;
        let start = NOW;
        let mut checks = Vec::new();
        while now < NOW + 7 * DAY {
            let since_start = Duration::from_secs(now - start);
            let waited = wait(last_check, now, since_start);
            if waited.is_zero() {
                checks.push(now);
                last_check = Some(now);
                now += 1;
            } else {
                now += waited.as_secs();
            }
        }
        assert_eq!(checks.len(), 7);
        assert!(checks.windows(2).all(|pair| pair[1] - pair[0] >= DAY));
        assert_eq!(checks[0], NOW + SETTLE_DELAY.as_secs());
    }

    const HOUR: u64 = 60 * 60;

    #[test]
    fn failed_checks_are_retried_after_1_2_4_hours_up_to_a_day() {
        let hours = (0..=8)
            .map(|failures| retry_interval(failures).as_secs() / HOUR)
            .collect::<Vec<_>>();
        assert_eq!(hours, [24, 1, 2, 4, 8, 16, 24, 24, 24]);
        assert_eq!(retry_interval(u32::MAX), UPDATE_INTERVAL);
    }

    #[test]
    fn after_a_failure_the_next_check_is_due_after_the_back_off() {
        let wait = |failures, since_last: u64| {
            wait_before_check(Some(NOW - since_last), failures, NOW, SETTLED, SETTLE_DELAY)
        };
        assert_eq!(wait(1, HOUR), Duration::ZERO);
        assert_eq!(wait(1, HOUR - 60), Duration::from_secs(60));
        assert_eq!(wait(3, 3 * HOUR), Duration::from_secs(HOUR));
        assert_eq!(wait(3, 4 * HOUR), Duration::ZERO);
        // A check that reached the publishers waits the whole day.
        assert_eq!(wait(0, 4 * HOUR), RECHECK_INTERVAL);
    }
}
