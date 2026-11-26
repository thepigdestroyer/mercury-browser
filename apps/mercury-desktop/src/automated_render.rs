// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Keeps a covered window rendering in automated runs (orbit-view brief D13): a window that is
//! covered renders only for `take_snapshot`, so a `--background` run never presents the frames that
//! the tab pictures (`tab_snapshots.rs`) are made from. With `MERCURY_AUTOMATED_RENDER_MS` set (50 to
//! 5000) a repeating timer takes a snapshot of the window that often. It counts in automated runs
//! only (`--screenshot`, `--background`, import); anywhere else it is ignored with a warning, and
//! without the variable nothing is created.
//!
//! "Automated" here is `Options::is_automated()` (`cli.rs`), which also counts `--import-from`.
//! `mercury_site_security::is_automated_run` counts only `--screenshot` and `--background`. The
//! difference is harmless for this driver (a snapshot has no side effect), but the two meanings
//! are not the same: do not use one where the other is meant.

use std::cell::Cell;
use std::fmt;
use std::time::Duration;

use mercury_ui::MainWindow;
use slint::{ComponentHandle, Timer, TimerMode};

/// The variable that asks for the driver: a whole number of milliseconds.
pub(crate) const PERIOD_VARIABLE: &str = "MERCURY_AUTOMATED_RENDER_MS";
/// The shortest period that is accepted (a snapshot of a large window takes several milliseconds).
pub(crate) const MIN_PERIOD_MS: u64 = 50;
/// The longest period that is accepted (a longer one would not keep a window rendering).
pub(crate) const MAX_PERIOD_MS: u64 = 5000;

/// Why a value of [`PERIOD_VARIABLE`] was ignored.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Notice {
    /// The run is a normal one: the variable is for automated runs only.
    NotAutomated,
    /// Not a whole number of milliseconds (empty, a fraction, a sign, units, words).
    NotANumber(String),
    /// A number, but outside 50 to 5000.
    OutOfRange(u64),
}

impl fmt::Display for Notice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAutomated => write!(
                formatter,
                "{PERIOD_VARIABLE} is ignored: it works only in automated runs \
                 (--screenshot, --background, import)"
            ),
            Self::NotANumber(text) => write!(
                formatter,
                "{PERIOD_VARIABLE}={text:?} is ignored: it must be a whole number of milliseconds"
            ),
            Self::OutOfRange(value) => write!(
                formatter,
                "{PERIOD_VARIABLE}={value} is ignored: it must be from {MIN_PERIOD_MS} to \
                 {MAX_PERIOD_MS} milliseconds"
            ),
        }
    }
}

/// What the variable asks for. `text` is its value (`None`: not set, which is neither a period
/// nor a notice). Surrounding white space is allowed; anything else that is not plain digits is
/// not a number.
pub(crate) fn read_period(
    automated: bool,
    text: Option<&str>,
) -> (Option<Duration>, Option<Notice>) {
    let Some(text) = text else {
        return (None, None);
    };
    if !automated {
        return (None, Some(Notice::NotAutomated));
    }
    let digits = text.trim();
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return (None, Some(Notice::NotANumber(text.to_owned())));
    }
    // All digits, so the only way to fail is a number that does not fit.
    let millis = digits.parse::<u64>().unwrap_or(u64::MAX);
    if (MIN_PERIOD_MS..=MAX_PERIOD_MS).contains(&millis) {
        (Some(Duration::from_millis(millis)), None)
    } else {
        (None, Some(Notice::OutOfRange(millis)))
    }
}

/// Starts the driver if `MERCURY_AUTOMATED_RENDER_MS` asks for it in an automated run. The timer
/// goes to the `timers` vector of `main.rs` and stops with the others.
pub(crate) fn start(window: &MainWindow, automated: bool) -> Option<Timer> {
    let value = std::env::var_os(PERIOD_VARIABLE);
    let text = value.as_ref().map(|value| value.to_string_lossy());
    start_with(window, automated, text.as_deref())
}

/// [`start`] with the value given (the tests do not touch the environment).
fn start_with(window: &MainWindow, automated: bool, text: Option<&str>) -> Option<Timer> {
    let (period, notice) = read_period(automated, text);
    if let Some(notice) = notice {
        log::warn!("{notice}");
    }
    let period = period?;
    log::info!(
        "automated run: rendering the window every {} ms",
        period.as_millis()
    );
    let window = window.as_weak();
    let reported = Cell::new(false);
    Some(repeat(period, move || {
        let Some(window) = window.upgrade() else {
            return;
        };
        // `take_snapshot` bypasses the occlusion check and presents a frame, which is what the
        // tab pictures need. One warning is enough if it cannot.
        if let Err(error) = window.window().take_snapshot()
            && !reported.replace(true)
        {
            log::warn!("the automated render could not take a snapshot: {error}");
        }
    }))
}

/// Calls `tick` every `period` until the returned timer is dropped.
fn repeat(period: Duration, mut tick: impl FnMut() + 'static) -> Timer {
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, period, move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        tick();
    });
    timer
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    fn period(automated: bool, text: Option<&str>) -> Option<u64> {
        read_period(automated, text)
            .0
            .map(|period| period.as_millis() as u64)
    }

    #[test]
    fn the_variable_not_being_set_asks_for_nothing_and_says_nothing() {
        assert_eq!(read_period(true, None), (None, None));
        assert_eq!(read_period(false, None), (None, None));
    }

    #[test]
    fn a_valid_period_in_an_automated_run_is_taken() {
        assert_eq!(
            read_period(true, Some("250")),
            (Some(Duration::from_millis(250)), None)
        );
        assert_eq!(period(true, Some("50")), Some(50), "the lowest value");
        assert_eq!(period(true, Some("5000")), Some(5000), "the highest value");
        assert_eq!(
            period(true, Some(" 250\n")),
            Some(250),
            "white space is fine"
        );
    }

    #[test]
    fn a_period_outside_50_to_5000_is_ignored_with_a_notice() {
        for (text, value) in [("49", 49), ("0", 0), ("5001", 5001), ("60000", 60000)] {
            assert_eq!(
                read_period(true, Some(text)),
                (None, Some(Notice::OutOfRange(value))),
                "{text}"
            );
        }
        assert_eq!(
            read_period(true, Some("99999999999999999999999")),
            (None, Some(Notice::OutOfRange(u64::MAX))),
            "a number that does not fit is out of range, not a crash"
        );
    }

    #[test]
    fn text_that_is_not_a_whole_number_of_milliseconds_is_ignored_with_a_notice() {
        for text in [
            "", " ", "abc", "250ms", "2.5", "-250", "+250", "0x10", "1 000",
        ] {
            assert_eq!(
                read_period(true, Some(text)),
                (None, Some(Notice::NotANumber(text.to_owned()))),
                "{text:?}"
            );
        }
    }

    #[test]
    fn outside_an_automated_run_the_variable_is_ignored_with_a_notice() {
        for text in ["250", "5001", "abc", ""] {
            assert_eq!(
                read_period(false, Some(text)),
                (None, Some(Notice::NotAutomated)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn every_notice_names_the_variable_and_what_was_wrong() {
        let not_automated = Notice::NotAutomated.to_string();
        assert!(not_automated.contains(PERIOD_VARIABLE));
        assert!(not_automated.contains("automated"));
        let text = Notice::NotANumber("abc".into()).to_string();
        assert!(text.contains(PERIOD_VARIABLE) && text.contains("\"abc\""));
        let range = Notice::OutOfRange(7).to_string();
        assert!(range.contains('7') && range.contains("50") && range.contains("5000"));
    }

    #[test]
    fn nothing_is_created_without_the_variable_or_outside_an_automated_run() {
        crate::settings::tests::init_slint();
        let window = MainWindow::new().expect("the main window");

        assert!(start_with(&window, true, None).is_none(), "not set");
        assert!(
            start_with(&window, false, Some("250")).is_none(),
            "not automated"
        );
        assert!(
            start_with(&window, true, Some("abc")).is_none(),
            "malformed"
        );
        assert!(start_with(&window, true, Some("49")).is_none(), "too short");
        assert!(start_with(&window, true, Some("250")).is_some(), "valid");
    }

    #[test]
    fn the_timer_ticks_every_period_until_it_is_dropped() {
        crate::settings::tests::init_slint();
        let ticks = Rc::new(Cell::new(0u32));
        let counted = Rc::clone(&ticks);
        let timer = repeat(Duration::from_millis(250), move || {
            counted.set(counted.get() + 1);
        });

        for expected in 1..=3 {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(250));
            slint::platform::update_timers_and_animations();
            assert_eq!(ticks.get(), expected);
        }

        drop(timer);
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1000));
        slint::platform::update_timers_and_animations();
        assert_eq!(ticks.get(), 3, "a dropped timer stays quiet");
    }

    #[test]
    fn a_tick_runs_inside_the_slint_callback_scope() {
        crate::settings::tests::init_slint();
        let inside = Rc::new(Cell::new(false));
        let seen = Rc::clone(&inside);
        let _timer = repeat(Duration::from_millis(100), move || {
            seen.set(crate::callback_scope::inside_slint_callback());
        });

        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(100));
        slint::platform::update_timers_and_animations();

        assert!(inside.get());
        assert!(!crate::callback_scope::inside_slint_callback());
    }
}
