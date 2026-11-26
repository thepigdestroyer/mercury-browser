// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Time-zone questions: `time in tokyo`, `3pm new york to london`.

use jiff::Timestamp;
use jiff::civil::{DateTime, Weekday};
use jiff::tz::{Offset, TimeZone};

use crate::zone_names::{lookup, offset_from_sign, offset_id};
use crate::{Answer, AnswerKind, Context};

const WEEKDAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The moment asked about: now, or a clock time on today's date in the source zone.
#[derive(Clone, Copy)]
enum Moment {
    Now,
    At(i8, i8),
}

/// The answer to a time or time-zone question (`query` is already trimmed and collapsed).
pub(crate) fn answer(query: &str, context: &Context) -> Option<Answer> {
    candidates(query)
        .iter()
        .find_map(|(moment, source, target)| build(*moment, source, target, query, context))
}

/// The readings of the question: `(moment, source place, target place)`; an empty source is the
/// local zone. A bare clock time followed by `to` or `in` may split the phrase in more than one
/// way (`3:00 am america/new_york to london`), so every reading is returned in order.
fn candidates(query: &str) -> Vec<(Moment, String, String)> {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower.split(' ').collect();
    let count = words.len();
    if count >= 3 && words[1] == "in" && matches!(words[0], "time" | "now") {
        return vec![(Moment::Now, String::new(), words[2..].join(" "))];
    }
    if count >= 2 && words[count - 1] == "time" {
        return vec![(Moment::Now, String::new(), words[..count - 1].join(" "))];
    }
    let mut found = Vec::new();
    for used in 1..=2 {
        if used > count {
            break;
        }
        let spoken = if used == 1 {
            words[0].to_string()
        } else if matches!(words[1], "am" | "pm") {
            format!("{}{}", words[0], words[1])
        } else {
            continue;
        };
        let Some(moment) = clock(&spoken) else {
            continue;
        };
        let rest = &words[used..];
        if let Some(at) = rest.iter().position(|word| matches!(*word, "to" | "in")) {
            let target = rest[at + 1..].join(" ");
            if !target.is_empty() {
                found.push((moment, rest[..at].join(" "), target));
            }
        }
    }
    found
}

fn build(
    moment: Moment,
    source: &str,
    target: &str,
    query: &str,
    context: &Context,
) -> Option<Answer> {
    let source_zone = if source.is_empty() {
        None
    } else {
        Some(zone_for(source)?.0)
    };
    let (zone, display) = zone_for(target)?;
    let instant = match moment {
        Moment::Now => Timestamp::from_second(context.now_unix_secs).ok()?,
        Moment::At(hour, minute) => {
            let zone = match source_zone {
                Some(zone) => zone,
                None => tzdb_zone(&context.local_zone)?,
            };
            let today = Timestamp::from_second(context.now_unix_secs)
                .ok()?
                .to_zoned(zone.clone())
                .date();
            DateTime::new(today.year(), today.month(), today.day(), hour, minute, 0, 0)
                .ok()?
                .to_zoned(zone)
                .ok()?
                .timestamp()
        }
    };
    let local = instant.to_zoned(zone);
    let (hour, minute) = (local.hour(), local.minute());
    let text = if context.clock24 {
        format!("{hour:02}:{minute:02}")
    } else {
        let period = if hour < 12 { "AM" } else { "PM" };
        let twelve = match hour % 12 {
            0 => 12,
            hour => hour,
        };
        format!("{twelve}:{minute:02} {period}")
    };
    let offset = offset_id(local.offset().seconds());
    let weekday = weekday_name(local.weekday());
    let month = month_name(local.month())?;
    let day = format!("{weekday} {} {month}", local.day());
    let detail = if display == offset {
        format!("{display} · {day}")
    } else {
        format!("{display} · {day} · {offset}")
    };
    Some(Answer {
        kind: AnswerKind::TimeZone,
        query: query.to_string(),
        text: text.clone(),
        copy: text,
        detail: Some(detail),
    })
}

/// The fixed offset for `utc+5:30`, otherwise the bundled IANA zone for the place.
fn zone_for(word: &str) -> Option<(TimeZone, String)> {
    let (id, display) = lookup(word)?;
    if let Some(seconds) = id.strip_prefix("UTC").and_then(offset_from_sign) {
        return Some((
            TimeZone::fixed(Offset::from_seconds(seconds).ok()?),
            display,
        ));
    }
    Some((tzdb_zone(&id)?, display))
}

/// A zone from the bundled IANA data (never the machine's), if the id is known.
fn tzdb_zone(id: &str) -> Option<TimeZone> {
    let (name, data) = jiff_tzdb::get(id)?;
    TimeZone::tzif(name, data).ok()
}

/// `now` (any clock), `noon`, `midnight`, `3pm`, `3:30pm`, `15:30`, if it is a valid time.
fn clock(text: &str) -> Option<Moment> {
    match text {
        "now" => return Some(Moment::Now),
        "noon" => return Some(Moment::At(12, 0)),
        "midnight" => return Some(Moment::At(0, 0)),
        _ => {}
    }
    let (body, meridian) = if let Some(body) = text.strip_suffix("am") {
        (body, Some(false))
    } else if let Some(body) = text.strip_suffix("pm") {
        (body, Some(true))
    } else {
        (text, None)
    };
    let (hour, minute, colon) = match body.split_once(':') {
        Some((hour, minute)) => (digits(hour)?, digits(minute)?, true),
        None => (digits(body)?, 0, false),
    };
    if minute > 59 {
        return None;
    }
    let hour = match meridian {
        Some(pm) if (1..=12).contains(&hour) => hour % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None if colon && hour <= 23 => hour,
        None => return None,
    };
    Some(Moment::At(hour, minute))
}

/// One or two ASCII digits.
fn digits(text: &str) -> Option<i8> {
    if (1..=2).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}

fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => WEEKDAY_NAMES[0],
        Weekday::Tuesday => WEEKDAY_NAMES[1],
        Weekday::Wednesday => WEEKDAY_NAMES[2],
        Weekday::Thursday => WEEKDAY_NAMES[3],
        Weekday::Friday => WEEKDAY_NAMES[4],
        Weekday::Saturday => WEEKDAY_NAMES[5],
        Weekday::Sunday => WEEKDAY_NAMES[6],
    }
}

fn month_name(month: i8) -> Option<&'static str> {
    let index = usize::try_from(month.checked_sub(1)?).ok()?;
    MONTH_NAMES.get(index).copied()
}
