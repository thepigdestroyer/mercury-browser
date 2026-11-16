//! Local calendar days for the history window and the clear dialog: grouping visits by day,
//! naming the days, and where "today" starts. The time zone is the system's, through
//! `localtime_r`/`mktime`.

/// A calendar day in the local time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct LocalDay {
    pub(crate) year: i32,
    /// 1–12.
    pub(crate) month: u32,
    /// 1–31.
    pub(crate) day: u32,
    /// 0 = Sunday … 6 = Saturday.
    pub(crate) weekday: u32,
}

/// A moment in local time: its day and its time of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalTime {
    pub(crate) day: LocalDay,
    pub(crate) hour: u32,
    pub(crate) minute: u32,
}

/// `ms` since the Unix epoch, in the local time zone.
pub(crate) fn local_time(ms: i64) -> LocalTime {
    let seconds = ms.div_euclid(1000) as libc::time_t;
    // SAFETY: `localtime_r` writes only into the `tm` it is given and reads the time zone,
    // which nothing in this process changes after start-up.
    let tm = unsafe {
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&seconds, &mut tm);
        tm
    };
    LocalTime {
        day: LocalDay {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            weekday: tm.tm_wday as u32,
        },
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
    }
}

/// When the local day containing `ms` began, in ms since the Unix epoch.
#[allow(
    clippy::useless_conversion,
    reason = "`time_t` is not 64 bits on every platform"
)]
pub(crate) fn start_of_day(ms: i64) -> i64 {
    let seconds = ms.div_euclid(1000) as libc::time_t;
    // SAFETY: as in `local_time`; `mktime` normalises the `tm` it is given.
    let start = unsafe {
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&seconds, &mut tm);
        tm.tm_hour = 0;
        tm.tm_min = 0;
        tm.tm_sec = 0;
        tm.tm_isdst = -1;
        libc::mktime(&mut tm)
    };
    i64::from(start) * 1000
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// How the history window heads a day: "Today", "Yesterday", or "Monday 16 November 2026".
/// `yesterday` is the day before `today`, from the caller (which knows the calendar).
pub(crate) fn day_label(day: LocalDay, today: LocalDay, yesterday: LocalDay) -> String {
    if day == today {
        return "Today".to_owned();
    }
    if day == yesterday {
        return "Yesterday".to_owned();
    }
    let weekday = WEEKDAYS.get(day.weekday as usize).copied().unwrap_or("");
    let month = MONTHS
        .get(day.month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("");
    format!("{weekday} {} {month} {}", day.day, day.year)
}

/// "14:05".
pub(crate) fn clock_label(time: LocalTime) -> String {
    format!("{:02}:{:02}", time.hour, time.minute)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u32, day: u32, weekday: u32) -> LocalDay {
        LocalDay {
            year,
            month,
            day,
            weekday,
        }
    }

    #[test]
    fn days_are_named_relative_to_today() {
        let today = day(2026, 11, 16, 1);
        let yesterday = day(2026, 11, 15, 0);

        assert_eq!(day_label(today, today, yesterday), "Today");
        assert_eq!(day_label(yesterday, today, yesterday), "Yesterday");
        assert_eq!(
            day_label(day(2026, 11, 12, 4), today, yesterday),
            "Thursday 12 November 2026"
        );
    }

    #[test]
    fn times_have_two_digit_hours_and_minutes() {
        assert_eq!(
            clock_label(LocalTime {
                day: day(2026, 1, 1, 4),
                hour: 9,
                minute: 5
            }),
            "09:05"
        );
    }

    #[test]
    fn a_day_starts_at_local_midnight() {
        let now = 1_794_831_334_759;
        let start = start_of_day(now);
        let midnight = local_time(start);

        assert!(start <= now && now - start < 25 * 3_600_000);
        assert_eq!((midnight.hour, midnight.minute), (0, 0));
        assert_eq!(midnight.day, local_time(now).day);
    }
}
