// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! HTTP dates: formatting `Date` and `Last-Modified`, and reading `If-Modified-Since`.

const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date of a day number from [`days_from_civil`]: (year, month, day).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `Tue, 17 Nov 2026 09:05:03 GMT` for a Unix time in whole seconds (Python's
/// `email.utils.formatdate(timestamp, usegmt=True)`).
pub fn format(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let seconds_of_day = unix_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday.
    let weekday = (days + 3).rem_euclid(7) as usize;
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        DAY_NAMES[weekday],
        day,
        MONTH_NAMES[(month - 1) as usize],
        year,
        seconds_of_day / 3600,
        seconds_of_day % 3600 / 60,
        seconds_of_day % 60
    )
}

/// A date read by [`parse`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedDate {
    /// The written date and time as if it were UTC, in Unix seconds.
    pub unix_seconds: i64,
    /// The zone offset in seconds, or `None` when the date has no zone (asctime) or says
    /// `-0000` ("unknown local time"); both are read as UTC, as Python did.
    pub offset_seconds: Option<i64>,
}

impl ParsedDate {
    /// Whether the date can be compared with a file's time: UTC, or no zone. Python's server
    /// ignored `If-Modified-Since` in any other zone, and so does this one.
    pub fn is_utc(&self) -> bool {
        matches!(self.offset_seconds, None | Some(0))
    }
}

fn number(text: &str) -> Option<i64> {
    if text.is_empty() || text.len() > 4 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn month_number(name: &str) -> Option<i64> {
    MONTH_NAMES
        .iter()
        .position(|month| month.eq_ignore_ascii_case(name))
        .map(|index| index as i64 + 1)
}

/// `GMT`, `UTC`, `UT`, `Z` or a numeric `+hhmm` / `-hhmm` zone; `Some(None)` for `-0000`.
fn zone_offset(zone: &str) -> Option<Option<i64>> {
    if ["GMT", "UTC", "UT", "Z"]
        .iter()
        .any(|name| name.eq_ignore_ascii_case(zone))
    {
        return Some(Some(0));
    }
    let (sign, digits) = match zone.split_at_checked(1)? {
        ("+", digits) => (1, digits),
        ("-", digits) => (-1, digits),
        _ => return None,
    };
    if digits.len() != 4 {
        return None;
    }
    let hhmm = number(digits)?;
    if hhmm == 0 && sign < 0 {
        return Some(None);
    }
    Some(Some(sign * ((hhmm / 100) * 3600 + (hhmm % 100) * 60)))
}

fn date(day: &str, month: &str, year: &str, time: &str, zone: Option<&str>) -> Option<ParsedDate> {
    let day = number(day)?;
    let month = month_number(month)?;
    let mut year = number(year)?;
    if year < 100 {
        year += if year > 68 { 1900 } else { 2000 };
    }
    let mut clock = time.split(':');
    let hour = number(clock.next()?)?;
    let minute = number(clock.next()?)?;
    let second = clock.next().map_or(Some(0), number)?;
    if clock.next().is_some() {
        return None;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days_in_month).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let offset_seconds = match zone {
        Some(zone) => zone_offset(zone)?,
        None => None,
    };
    Some(ParsedDate {
        unix_seconds: days_from_civil(year, month, day) * 86_400
            + hour * 3600
            + minute * 60
            + second,
        offset_seconds,
    })
}

/// Reads a date in one of the three forms HTTP allows (RFC 9110, section 5.6.7): IMF-fixdate
/// (`Sun, 06 Nov 1994 08:49:37 GMT`, also with a numeric zone), RFC 850
/// (`Sunday, 06-Nov-94 08:49:37 GMT`) and asctime (`Sun Nov  6 08:49:37 1994`). `None` for
/// anything else, which `If-Modified-Since` then ignores.
pub fn parse(text: &str) -> Option<ParsedDate> {
    let words: Vec<&str> = text.split_whitespace().collect();
    match words.as_slice() {
        [weekday, day, month, year, time, zone] if weekday.ends_with(',') => {
            date(day, month, year, time, Some(zone))
        }
        [weekday, day_month_year, time, zone] if weekday.ends_with(',') => {
            let mut parts = day_month_year.split('-');
            let (day, month, year) = (parts.next()?, parts.next()?, parts.next()?);
            if parts.next().is_some() {
                return None;
            }
            date(day, month, year, time, Some(zone))
        }
        [_weekday, month, day, time, year] => date(day, month, year, time, None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_python() {
        assert_eq!(format(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(format(784_111_777), "Sun, 06 Nov 1994 08:49:37 GMT");
        assert_eq!(format(1_795_424_703), "Mon, 23 Nov 2026 09:05:03 GMT");
        assert_eq!(format(951_782_400), "Tue, 29 Feb 2000 00:00:00 GMT");
    }

    #[test]
    fn round_trips() {
        for seconds in [0, 1, 86_399, 951_782_400, 1_795_424_703, 4_102_444_800] {
            let parsed = parse(&format(seconds)).expect("parses");
            assert_eq!(parsed.unix_seconds, seconds);
            assert_eq!(parsed.offset_seconds, Some(0));
        }
    }

    #[test]
    fn reads_the_three_http_forms() {
        let expected = Some(784_111_777);
        assert_eq!(
            parse("Sun, 06 Nov 1994 08:49:37 GMT").map(|d| d.unix_seconds),
            expected
        );
        assert_eq!(
            parse("Sunday, 06-Nov-94 08:49:37 GMT").map(|d| d.unix_seconds),
            expected
        );
        let asctime = parse("Sun Nov  6 08:49:37 1994").expect("asctime");
        assert_eq!(asctime.unix_seconds, 784_111_777);
        assert_eq!(asctime.offset_seconds, None);
        assert!(asctime.is_utc());
    }

    #[test]
    fn zones() {
        assert_eq!(
            parse("Sun, 06 Nov 1994 08:49:37 +0130").and_then(|d| d.offset_seconds),
            Some(5400)
        );
        assert!(!parse("Sun, 06 Nov 1994 08:49:37 -0500").unwrap().is_utc());
        assert!(parse("Sun, 06 Nov 1994 08:49:37 -0000").unwrap().is_utc());
        assert!(parse("Sun, 06 Nov 1994 08:49:37 utc").unwrap().is_utc());
    }

    #[test]
    fn refuses_what_python_refuses() {
        for text in [
            "",
            "yesterday",
            "Sun, 31 Feb 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 24:00:00 GMT",
            "Sun, 06 Nov 1994 GMT",
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:37 EST",
            "Sun, 06 Nov 1994 08:49:37 Mars",
            "Sun, 06 Nov 1994 08:49:37:00 GMT",
            "06 Nov 1994 08:49:37 GMT",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }
}
