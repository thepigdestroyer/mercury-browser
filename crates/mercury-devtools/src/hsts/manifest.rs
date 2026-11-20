// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The manifest that sits beside the data file (`hsts_preload.manifest.json`): where the list came
//! from, when, and what it contains, so that a refresh can be audited and `check` can tell that the
//! manifest and the data still belong together. Also the two small things the manifest needs and
//! the standard library lacks: a calendar [`Date`] and a SHA-256 in hex ([`sha256_hex`]); the
//! tool has no `chrono` or `time` dependency for these.
//!
//! The manifest is JSON written through `serde_json::Value` with its keys in sorted order, two
//! spaces of indent and a final newline, so the same data always gives the same text and a
//! refresh shows up in `git diff` as the lines that really changed.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::fstmap::Preload;

/// The `format` field of every manifest this tool writes: what the data file contains.
pub const FORMAT: &str =
    "fst-map-v3; key = host name (lower-case ASCII); value = 2 * rank + include_subdomains";

/// Dates beyond this many days either side of 1970-01-01 (about 274,000 years) are clamped by
/// [`Date::from_unix_days`], so that the year always fits and nothing overflows.
const MAX_DAYS: i64 = 100_000_000;

/// Days from 0000-03-01 (the start of the "March year", see [`days_from_civil`]) to 1970-01-01.
const DAYS_TO_EPOCH: i64 = 719_468;

/// Days in a 400-year cycle of the Gregorian calendar (the cycle repeats exactly).
const DAYS_PER_ERA: i64 = 146_097;

const MONTH_NAMES: [&str; 12] = [
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

/// A calendar date in the proleptic Gregorian calendar (UTC, no time of day).
///
/// The fields are public so a date can be written down directly; one that is not a real date
/// (month 13) does not panic anywhere, it just converts to a meaningless day number. [`Date::parse`]
/// only ever returns real dates. Ordering is chronological (year, then month, then day).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    /// Parses `YYYY-MM-DD`: four digits, a dash, two digits, a dash, two digits, and a date that
    /// exists (30 February and a 13th month are errors; a leap day is only valid in a leap year).
    /// Nothing around the text is trimmed and no other spelling (`2026-1-5`, `20261120`) is
    /// accepted: the manifest and the command line must agree on one form.
    pub fn parse(text: &str) -> Result<Date, String> {
        let bad_form =
            || format!("{text:?} is not a date written YYYY-MM-DD (for example 2026-11-20)");
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(bad_form());
        }
        let digits = |from: usize, to: usize| -> Option<u32> {
            let part = &bytes[from..to];
            part.iter().all(u8::is_ascii_digit).then(|| {
                part.iter()
                    .fold(0, |sum, byte| sum * 10 + u32::from(byte - b'0'))
            })
        };
        let (Some(year), Some(month), Some(day)) = (digits(0, 4), digits(5, 7), digits(8, 10))
        else {
            return Err(bad_form());
        };
        let Ok(year) = i32::try_from(year) else {
            return Err(bad_form());
        };
        if !(1..=12).contains(&month) {
            return Err(format!(
                "{text:?} is not a real date: there is no month {month} (months are 01 to 12)"
            ));
        }
        let length = days_in_month(year, month);
        if !(1..=length).contains(&day) {
            return Err(format!(
                "{text:?} is not a real date: {} {year} has {length} days, and there is no day {day}",
                MONTH_NAMES[month as usize - 1]
            ));
        }
        Ok(Date { year, month, day })
    }

    /// The date `days` days after 1970-01-01 (before it when negative). Absurd values (more than
    /// 100 million days either way) are clamped to the edge, so this never panics or overflows.
    pub fn from_unix_days(days: i64) -> Date {
        civil_from_days(days.clamp(-MAX_DAYS, MAX_DAYS))
    }

    /// The number of days from 1970-01-01 to this date (negative before it).
    pub fn unix_days(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// Today's date in UTC, from the system clock. A clock before 1970 gives 1970-01-01; this
    /// never fails.
    pub fn today_utc() -> Date {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        Date::from_unix_days(i64::try_from(seconds / 86_400).unwrap_or(i64::MAX))
    }

    /// How many days this date is after `earlier` (negative when `earlier` is the later one).
    pub fn days_since(self, earlier: Date) -> i64 {
        self.unix_days() - earlier.unix_days()
    }
}

impl fmt::Display for Date {
    /// `YYYY-MM-DD`, the form [`Date::parse`] reads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a calendar date.
///
/// Derived on the "March year": if the year starts on 1 March, the leap day is the last day of the
/// year, so the days before a month follow a fixed pattern (`(153 * m + 2) / 5` for month number
/// `m` counted from March = 0 gives 0, 31, 61, 92, 122, 153, 184, 214, 245, 275, 306, 337), and
/// the Gregorian leap rule is the same arithmetic every year (`y / 4 - y / 100 + y / 400`) with
/// the 400-year cycle of 146,097 days repeating exactly. A date in January or February belongs
/// to the March year that started the year before. The tests check it against known day numbers
/// and against a day-by-day walk through 130 years.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let month = i64::from(month);
    let march_year = i64::from(year) - i64::from(month <= 2);
    let era = march_year.div_euclid(400);
    let year_of_era = march_year.rem_euclid(400);
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * DAYS_PER_ERA + day_of_era - DAYS_TO_EPOCH
}

/// The inverse of [`days_from_civil`]: split the days into 400-year cycles, find the year inside
/// the cycle (removing one day per 4 years, adding back one per 100 and one per 400 so the
/// estimate `/ 365` lands on the right year), then the month from the day of the March year.
fn civil_from_days(days: i64) -> Date {
    let shifted = days + DAYS_TO_EPOCH;
    let era = shifted.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted.rem_euclid(DAYS_PER_ERA);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    Date {
        // Clamped by `Date::from_unix_days`, so the year is within 300,000 of 1970 and the month
        // and day are small positive numbers.
        year: i32::try_from(year).unwrap_or(i32::MAX),
        month: u32::try_from(month).unwrap_or(1),
        day: u32::try_from(day).unwrap_or(1),
    }
}

/// The SHA-256 of `bytes` as 64 lower-case hex digits.
pub fn sha256_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    hex
}

/// Where the data came from. Every field but `name` may be unknown (`None`, written as `null`):
/// the manifest of the file copied from Servo has nothing but a name.
///
/// JSON keys: `name url ref retrieved input_sha256 objects force_https` (`git_ref` is `ref`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// A human description of the origin.
    pub name: String,
    /// The URL the input was downloaded from.
    pub url: Option<String>,
    /// The Chromium revision asked for (`main`, or a commit hash that pins it).
    pub git_ref: Option<String>,
    /// The day the input was retrieved (UTC).
    pub retrieved: Option<Date>,
    /// The SHA-256 of the input exactly as read (before base64 decoding).
    pub input_sha256: Option<String>,
    /// How many objects the input's `entries` list held.
    pub objects: Option<usize>,
    /// How many of them were `force-https` entries.
    pub force_https: Option<usize>,
}

/// The manifest of one data file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// What the data file contains: [`FORMAT`] for every file this tool writes.
    pub format: String,
    /// The number of hosts in the data file.
    pub entries: usize,
    /// How many of them include their subdomains.
    pub include_subdomains: usize,
    /// The SHA-256 of the data file.
    pub output_sha256: String,
    pub source: Source,
    /// The tool that wrote the data: `hsts-preload <version>`, or a plain sentence when the data
    /// was not made by the tool.
    pub tool: String,
}

impl Manifest {
    /// The manifest for `bytes`, the data file, which has been opened as `preload`: the counts and
    /// hash come from the data, so they cannot disagree with it.
    pub fn for_data(bytes: &[u8], preload: &Preload, source: Source) -> Manifest {
        Manifest {
            format: FORMAT.to_string(),
            entries: preload.len(),
            include_subdomains: preload.include_subdomains_count(),
            output_sha256: sha256_hex(bytes),
            source,
            tool: format!("hsts-preload {}", env!("CARGO_PKG_VERSION")),
        }
    }

    /// The manifest as JSON text: keys sorted (the objects are filled in key order, so this holds
    /// even if another crate in the build turns on `serde_json`'s `preserve_order`), two spaces of
    /// indent, `null` for an unknown field, and a final newline.
    pub fn to_json(&self) -> String {
        let mut source = Map::new();
        source.insert(
            "force_https".into(),
            optional_count(self.source.force_https),
        );
        source.insert(
            "input_sha256".into(),
            optional_text(self.source.input_sha256.as_deref()),
        );
        source.insert("name".into(), Value::from(self.source.name.as_str()));
        source.insert("objects".into(), optional_count(self.source.objects));
        source.insert("ref".into(), optional_text(self.source.git_ref.as_deref()));
        source.insert(
            "retrieved".into(),
            self.source
                .retrieved
                .map_or(Value::Null, |date| Value::from(date.to_string())),
        );
        source.insert("url".into(), optional_text(self.source.url.as_deref()));

        let mut manifest = Map::new();
        manifest.insert("entries".into(), Value::from(self.entries as u64));
        manifest.insert("format".into(), Value::from(self.format.as_str()));
        manifest.insert(
            "include_subdomains".into(),
            Value::from(self.include_subdomains as u64),
        );
        manifest.insert(
            "output_sha256".into(),
            Value::from(self.output_sha256.as_str()),
        );
        manifest.insert("source".into(), Value::Object(source));
        manifest.insert("tool".into(), Value::from(self.tool.as_str()));

        let mut text = serde_json::to_string_pretty(&Value::Object(manifest))
            .expect("a JSON value made of strings, numbers and null always serialises");
        text.push('\n');
        text
    }

    /// Reads a manifest. A field that is missing or has the wrong type is an error that names it
    /// (`source.retrieved`, `entries`); the optional `source` fields may be `null` but must be
    /// present, so a truncated or hand-damaged manifest is noticed. A key this version does not
    /// know is ignored.
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("the manifest is not valid JSON: {error}"))?;
        let Value::Object(top) = &value else {
            return Err(format!(
                "the manifest must be a JSON object, found {}",
                kind(&value)
            ));
        };
        let top = Fields {
            map: top,
            prefix: "",
        };
        let source_value = top.value("source")?;
        let Value::Object(source_map) = source_value else {
            return Err(format!(
                "manifest field `source` must be an object, found {}",
                kind(source_value)
            ));
        };
        let source = Fields {
            map: source_map,
            prefix: "source.",
        };
        Ok(Manifest {
            format: top.text("format")?,
            entries: top.count("entries")?,
            include_subdomains: top.count("include_subdomains")?,
            output_sha256: top.text("output_sha256")?,
            source: Source {
                name: source.text("name")?,
                url: source.optional_text("url")?,
                git_ref: source.optional_text("ref")?,
                retrieved: source.optional_date("retrieved")?,
                input_sha256: source.optional_text("input_sha256")?,
                objects: source.optional_count("objects")?,
                force_https: source.optional_count("force_https")?,
            },
            tool: top.text("tool")?,
        })
    }
}

fn optional_text(text: Option<&str>) -> Value {
    text.map_or(Value::Null, Value::from)
}

fn optional_count(count: Option<usize>) -> Value {
    count.map_or(Value::Null, |count| Value::from(count as u64))
}

/// What a JSON value is, for an error message ("found a string").
fn kind(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(_) => "a boolean".to_string(),
        Value::Number(number) => format!("the number {number}"),
        Value::String(_) => "a string".to_string(),
        Value::Array(_) => "a list".to_string(),
        Value::Object(_) => "an object".to_string(),
    }
}

/// One JSON object being read, with the prefix its field names carry in error messages.
struct Fields<'a> {
    map: &'a Map<String, Value>,
    prefix: &'static str,
}

impl Fields<'_> {
    fn value(&self, key: &str) -> Result<&Value, String> {
        self.map
            .get(key)
            .ok_or_else(|| format!("the manifest has no field `{}{key}`", self.prefix))
    }

    fn wrong(&self, key: &str, wanted: &str, found: &Value) -> String {
        format!(
            "manifest field `{}{key}` must be {wanted}, found {}",
            self.prefix,
            kind(found)
        )
    }

    fn text(&self, key: &str) -> Result<String, String> {
        match self.value(key)? {
            Value::String(text) => Ok(text.clone()),
            other => Err(self.wrong(key, "a string", other)),
        }
    }

    fn count(&self, key: &str) -> Result<usize, String> {
        let value = self.value(key)?;
        value
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .ok_or_else(|| self.wrong(key, "a whole number of 0 or more", value))
    }

    fn optional_text(&self, key: &str) -> Result<Option<String>, String> {
        match self.value(key)? {
            Value::Null => Ok(None),
            Value::String(text) => Ok(Some(text.clone())),
            other => Err(self.wrong(key, "a string or null", other)),
        }
    }

    fn optional_count(&self, key: &str) -> Result<Option<usize>, String> {
        match self.value(key)? {
            Value::Null => Ok(None),
            value => value
                .as_u64()
                .and_then(|number| usize::try_from(number).ok())
                .map(Some)
                .ok_or_else(|| self.wrong(key, "a whole number of 0 or more or null", value)),
        }
    }

    fn optional_date(&self, key: &str) -> Result<Option<Date>, String> {
        match self.value(key)? {
            Value::Null => Ok(None),
            Value::String(text) => Date::parse(text).map(Some).map_err(|why| {
                format!(
                    "manifest field `{}{key}` must be a date written YYYY-MM-DD or null: {why}",
                    self.prefix
                )
            }),
            other => Err(self.wrong(key, "a date written YYYY-MM-DD or null", other)),
        }
    }
}
