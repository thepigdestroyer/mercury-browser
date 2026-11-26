// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Suggested file names for a saved PDF or screenshot.
//!
//! Pure: the clock is passed in, so the names can be shown before anything is written.

use std::time::{SystemTime, UNIX_EPOCH};

/// The longest name part, in characters, taken from a title or a host.
const MAX_PART_CHARS: usize = 100;

/// The name used when neither the title nor the host gives one.
const FALLBACK_NAME: &str = "Page";

/// A file name for a PDF of a page: the title, else the host, plus `.pdf`.
pub fn suggested_pdf_name(title: &str, host: &str) -> String {
    let base = clean(title)
        .or_else(|| clean(host))
        .unwrap_or_else(|| FALLBACK_NAME.to_owned());
    format!("{base}.pdf")
}

/// A file name for a screenshot: `Screenshot <host> <UTC date and time>.png`.
pub fn suggested_screenshot_name(host: &str, now: SystemTime) -> String {
    let stamp = utc_stamp(now);
    match clean(host) {
        Some(host) => format!("Screenshot {host} {stamp}.png"),
        None => format!("Screenshot {stamp}.png"),
    }
}

/// Turns a title or host into one safe file name part, or `None` if nothing is left.
///
/// Control characters and bidirectional or invisible formatting marks are dropped, path
/// separators and colons become spaces, white space is collapsed, leading and trailing dots and
/// spaces are removed (so the name is never hidden), and the part is cut to `MAX_PART_CHARS`.
fn clean(raw: &str) -> Option<String> {
    let kept: String = raw
        .chars()
        .filter(|&c| !c.is_control() && !is_hidden_format(c))
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let joined = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let limited: String = joined.chars().take(MAX_PART_CHARS).collect();
    let trimmed = limited.trim_matches(|c: char| c == '.' || c == ' ');
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn is_hidden_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{200B}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

/// The UTC date and time as `YYYY-MM-DD HH.MM.SS`; dots, because colons are not allowed in names.
fn utc_stamp(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let in_day = secs.rem_euclid(86_400);
    let (hour, minute, second) = (in_day / 3_600, in_day % 3_600 / 60, in_day % 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}.{minute:02}.{second:02}")
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_based_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_based_month + 2) / 5 + 1;
    let month = if march_based_month < 10 {
        march_based_month + 3
    } else {
        march_based_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(suggested_pdf_name("", "example.com"), "example.com.pdf");
        assert_eq!(suggested_pdf_name("", ""), "Page.pdf");
        assert_eq!(suggested_pdf_name("", "  ..  "), "Page.pdf");

        let separators = suggested_pdf_name("a/b:c\0d", "");
        assert!(!separators.contains('/'));
        assert!(!separators.contains(':'));
        assert!(!separators.contains('\0'));
        assert_eq!(separators, "a b cd.pdf");

        let long = suggested_pdf_name(&"x".repeat(500), "");
        assert_eq!(long.strip_suffix(".pdf").map(str::len), Some(100));

        assert_eq!(suggested_pdf_name("\u{202e}evil", ""), "evil.pdf");
        assert!(!suggested_pdf_name(".hidden", "").starts_with('.'));
        assert_eq!(suggested_pdf_name("CON", ""), "CON.pdf");
        assert_eq!(suggested_pdf_name("Café 🦀", ""), "Café 🦀.pdf");

        for title in ["", "...", "\u{200b}", "\u{feff}"] {
            let name = suggested_pdf_name(title, "");
            assert!(name.len() > ".pdf".len());
            assert!(name.ends_with(".pdf"));
        }
    }

    #[test]
    fn screenshot_name_has_host_and_time() {
        let now = UNIX_EPOCH + Duration::from_secs(1_795_356_189);
        assert_eq!(
            suggested_screenshot_name("example.com", now),
            "Screenshot example.com 2026-11-22 14.03.09.png"
        );
        assert_eq!(
            suggested_screenshot_name("", now),
            "Screenshot 2026-11-22 14.03.09.png"
        );
    }

    #[test]
    fn civil_dates_match_the_calendar() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_454), (2026, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
