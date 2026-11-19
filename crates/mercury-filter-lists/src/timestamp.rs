// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! UTC timestamps in the one form the manifest uses, `YYYY-MM-DDTHH:MM:SSZ`.

/// Seconds since the Unix epoch for `text` in the form `2026-11-15T20:46:51Z`, or `None` for
/// anything else (other forms, offsets, fractions, dates before 1970 or impossible dates).
pub fn parse_utc(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u64> {
        let digits = &bytes[range];
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        Some(
            digits
                .iter()
                .fold(0, |value, digit| value * 10 + u64::from(digit - b'0')),
        )
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if year < 1970
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// `seconds` since the Unix epoch in the form [`parse_utc`] reads.
pub fn format_utc(seconds: u64) -> String {
    let (year, month, day) = civil_from_days(seconds / 86_400);
    let rest = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The calendar date of `days` since 1970-01-01 (proleptic Gregorian).
pub fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let mut year = 1970;
    let mut days = days;
    loop {
        let length = if is_leap(year) { 366 } else { 365 };
        if days < length {
            break;
        }
        days -= length;
        year += 1;
    }
    let mut month = 1;
    while days >= days_in_month(year, month) {
        days -= days_in_month(year, month);
        month += 1;
    }
    (year, month, days + 1)
}

fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let years: u64 = (1970..year)
        .map(|year| if is_leap(year) { 366 } else { 365 })
        .sum();
    let months: u64 = (1..month).map(|month| days_in_month(year, month)).sum();
    years + months + day - 1
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_dates_parse_to_unix_seconds() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2026-11-15T20:46:51Z"), Some(1_794_775_611));
        assert_eq!(parse_utc("2024-02-29T12:00:00Z"), Some(1_709_208_000));
    }

    #[test]
    fn other_forms_and_impossible_dates_are_refused() {
        for text in [
            "",
            "2026-11-15",
            "2026-11-15 20:46:51Z",
            "2026-11-15T20:46:51",
            "2026-11-15T20:46:51+00:00",
            "2026-11-15T20:46:51.5Z",
            "2026-13-01T00:00:00Z",
            "2026-02-30T00:00:00Z",
            "2025-02-29T00:00:00Z",
            "2026-11-15T24:00:00Z",
            "2026-11-15T20:60:00Z",
            "1969-12-31T23:59:59Z",
            "+026-11-15T20:46:51Z",
            "２026-11-15T20:46:51Z",
        ] {
            assert_eq!(parse_utc(text), None, "{text:?}");
        }
    }

    #[test]
    fn formatting_reads_back() {
        for seconds in [0, 1_709_208_000, 1_794_775_611, 4_102_444_799] {
            assert_eq!(parse_utc(&format_utc(seconds)), Some(seconds));
        }
        assert_eq!(format_utc(1_794_775_611), "2026-11-15T20:46:51Z");
    }
}
