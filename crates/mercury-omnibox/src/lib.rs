// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Answers for the address bar: a calculator, unit conversion and time-zone conversion.
//!
//! [`answer`] takes the text typed in the address bar and either returns an [`Answer`] for the
//! suggestion popup to show as a row, or `None`, which means "this is not for me" (the text is a
//! search or an address as before). It never panics and reads nothing but its arguments: the clock
//! and the machine's time zone are in [`Context`]. Choosing an answer row copies [`Answer::copy`];
//! it navigates nowhere and the caller must not store it in history.

#![warn(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod calc;
mod format;
mod guard;
mod units;
mod zone_names;
mod zones;

/// The longest input looked at, in bytes; a longer text gets no answer.
pub const MAX_INPUT_BYTES: usize = 256;

/// What [`answer`] needs from the outside world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    /// "Now", in seconds since the Unix epoch.
    pub now_unix_secs: i64,
    /// The machine's time zone as an IANA id (`Europe/London`); `UTC` if the caller cannot tell.
    pub local_zone: String,
    /// Show times as `20:00` rather than `8:00 PM`.
    pub clock24: bool,
}

/// What kind of answer a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerKind {
    /// An arithmetic expression: `2*(3+4)` is `14`.
    Calculation,
    /// A unit conversion: `5 km to mi`.
    Conversion,
    /// A time or a time-zone conversion: `3pm new york to london`.
    TimeZone,
}

/// One answer, as the popup shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// What kind of answer it is.
    pub kind: AnswerKind,
    /// The input, trimmed and with runs of white space collapsed: the row's small label.
    pub query: String,
    /// The answer as shown (`1,024`, `3.10685596119 mi`, `8:00 PM`).
    pub text: String,
    /// What choosing the row puts on the clipboard (`1024`, `3.10685596119 mi`, `8:00 PM`).
    pub copy: String,
    /// A second line: the place, date and offset of a time answer; `None` otherwise.
    pub detail: Option<String>,
}

/// The answer for `input`, if it is a time-zone question, a unit conversion or arithmetic.
///
/// Tried in that order; the first that understands the text wins.
pub fn answer(input: &str, context: &Context) -> Option<Answer> {
    if input.len() > MAX_INPUT_BYTES {
        return None;
    }
    let query = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() {
        return None;
    }
    if let Some(found) = zones::answer(&query, context) {
        return Some(found);
    }
    if let Some((value, unit)) = units::convert(&query) {
        let (shown, copy) = format::number(value)?;
        return Some(Answer {
            kind: AnswerKind::Conversion,
            text: format!("{shown} {unit}"),
            copy: format!("{copy} {unit}"),
            query,
            detail: None,
        });
    }
    if guard::not_maths(&query) {
        return None;
    }
    let value = calc::evaluate(&query)?;
    let (shown, copy) = format::number(value)?;
    Some(Answer {
        kind: AnswerKind::Calculation,
        query,
        text: shown,
        copy,
        detail: None,
    })
}
