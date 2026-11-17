//! When the next automatic update check is due, as a pure function of times.
//!
//! Automatic checks run at most once every [`UPDATE_INTERVAL`] (24 hours), counted from the
//! start of the last check whatever its outcome, and never before [`SETTLE_DELAY`] after
//! start-up. A check is also due when the last one is recorded in the future (the clock was
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

/// How long to wait before the next automatic check, from `now` (seconds since the Unix
/// epoch). `last_check` is when the last check started; `since_start` is how long this run has
/// been going. `Duration::ZERO` means a check is due now. The result is never longer than
/// [`RECHECK_INTERVAL`]: the caller asks again after waiting.
pub fn wait_before_check(
    last_check: Option<u64>,
    now: u64,
    since_start: Duration,
    settle_delay: Duration,
) -> Duration {
    let settling = settle_delay.saturating_sub(since_start);
    let until_due = match last_check {
        Some(last) if last <= now => {
            UPDATE_INTERVAL.saturating_sub(Duration::from_secs(now - last))
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
        wait_before_check(last_check, now, since_start, SETTLE_DELAY)
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
            wait_before_check(Some(NOW - 2 * DAY), NOW, Duration::ZERO, Duration::ZERO),
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
}
