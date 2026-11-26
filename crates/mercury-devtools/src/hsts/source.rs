// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Chromium's HSTS list (`net/http/transport_security_state_static.json`) as [`Entry`] values.
//!
//! The file is JSON with `//` comment lines, and Gitiles serves it as base64 (`?format=TEXT`).
//! This reader is lenient about the surface (comments, trailing commas, base64 line breaks, a byte
//! order mark) and strict about the content: anything it does not understand is an error that
//! names the entry, never a silent drop, because the output is security data that Servo reads
//! without complaint (it treats a bad file as an empty list).
//!
//! What is kept (decisions D3 to D5 of the lane brief):
//! - an object of `"entries"` is an HSTS entry when its `mode` is `"force-https"`; `policy` is
//!   ignored (the list Servo 0.8.0 shipped keeps Chromium's `"policy": "test"` host);
//! - `include_subdomains` absent means `false`; a pin-only object (no `mode`) is skipped and
//!   counted, and its name is not checked;
//! - the name of every kept entry must be a plain host name ([`valid_host`]);
//! - a host listed twice is merged, `include_subdomains` winning, and the merge is counted;
//! - keys this reader does not know are counted in [`SourceReport::unknown_keys`], not refused:
//!   a new key at Chromium is worth a look in the report, not a failed refresh.
//!
//! Nothing here reads a file or the network; the caller hands over bytes.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry as MapEntry;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use super::Entry;

/// Keys of an `entries` object that Chromium documents (pinning and Expect-CT keys included: they
/// are ignored, but they are not news).
const KNOWN_KEYS: [&str; 10] = [
    "name",
    "policy",
    "mode",
    "include_subdomains",
    "include_subdomains_for_pinning",
    "pins",
    "expect_ct",
    "expect_ct_report_uri",
    "expect_staple",
    "expect_staple_report_uri",
];
/// The `mode` of an HSTS entry.
const MODE_FORCE_HTTPS: &str = "force-https";
/// DNS limits: a whole name, and one label.
const MAX_HOST_BYTES: usize = 253;
const MAX_LABEL_BYTES: usize = 63;
/// A name longer than this is cut when an error message quotes it.
const SHOWN_CHARS: usize = 100;

/// What the reader saw in the file, for the build summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceReport {
    /// Items of `"entries"`.
    pub objects: usize,
    /// Objects whose `mode` is `"force-https"` (before duplicates are merged).
    pub force_https: usize,
    /// Objects without `"force-https"`: pin-only entries, or another mode. `objects` is always
    /// `force_https + skipped`.
    pub skipped: usize,
    /// Hosts that were listed more than once (each extra listing counts once).
    pub merged_duplicates: usize,
    /// Keys of `entries` objects that are not in the documented set, with how often they occur.
    pub unknown_keys: BTreeMap<String, usize>,
}

/// The result of reading the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// Sorted by host and unique: `force_https - merged_duplicates` entries.
    pub entries: Vec<Entry>,
    pub report: SourceReport,
}

/// Decodes the standard-alphabet base64 that Gitiles serves for `?format=TEXT`. ASCII whitespace
/// (the line breaks Gitiles adds, a trailing newline) is ignored; padding is required, as Gitiles
/// writes it, so a download cut short does not decode as a shorter file.
pub fn decode_base64(text: &[u8]) -> Result<Vec<u8>, String> {
    let compact: Vec<u8> = text
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    STANDARD
        .decode(&compact)
        .map_err(|error| format!("the input is not base64: {error} (offsets ignore white space)"))
}

/// Blanks the comments out of JSON text and drops trailing commas, in one pass that knows about
/// strings (`"http://x"` stays whole).
///
/// - `//` to the end of the line and `/* ... */` are replaced by one space per byte; every
///   newline stays. The result is exactly as long as the input and every line and column is
///   unchanged, so a position that `serde_json` reports points into the original text.
/// - A `,` that follows a value and is followed, after white space and comments, by `]` or `}`
///   outside a string is replaced by a space. A comma with no value before it (`[,]`, `[1,,]`)
///   is left alone, so it is still an error.
///
/// A string or block comment that is still open at the end of the text is left as it is here;
/// [`parse_chromium`] reports it as an error with its position.
pub fn strip_comments(text: &str) -> String {
    scan(text).0
}

/// [`strip_comments`] and, when the text ends inside a string or a block comment, a sentence
/// saying where that string or comment began.
fn scan(text: &str) -> (String, Option<String>) {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    // Index in `out` of a comma that only white space and comments have followed so far.
    let mut comma_at: Option<usize> = None;
    // The last character that was neither white space nor part of a comment.
    let mut last: Option<char> = None;
    let mut problem = None;
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => {
                comma_at = None;
                last = Some('"');
                out.push('"');
                let mut closed = false;
                while let Some((_, inner)) = chars.next() {
                    out.push(inner);
                    if inner == '\\' {
                        if let Some((_, escaped)) = chars.next() {
                            out.push(escaped);
                        }
                    } else if inner == '"' {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    problem = Some(format!(
                        "the string that starts at {} is never closed",
                        position(text, at)
                    ));
                }
            }
            '/' if matches!(chars.peek(), Some((_, '/'))) => {
                out.push(' ');
                while let Some(&(_, next)) = chars.peek() {
                    if next == '\n' || next == '\r' {
                        break;
                    }
                    blank(&mut out, next);
                    chars.next();
                }
            }
            '/' if matches!(chars.peek(), Some((_, '*'))) => {
                chars.next();
                out.push_str("  ");
                let mut closed = false;
                let mut after_star = false;
                for (_, inner) in chars.by_ref() {
                    blank(&mut out, inner);
                    if after_star && inner == '/' {
                        closed = true;
                        break;
                    }
                    after_star = inner == '*';
                }
                if !closed {
                    problem = Some(format!(
                        "the block comment that starts at {} is never closed",
                        position(text, at)
                    ));
                }
            }
            ',' => {
                // Only a comma that follows a value can be a trailing one: `[,]` and `[1,,]`
                // stay broken, so serde reports them.
                comma_at = match last {
                    None | Some('[' | '{' | ',' | ':') => None,
                    Some(_) => Some(out.len()),
                };
                last = Some(',');
                out.push(',');
            }
            ']' | '}' => {
                if let Some(comma) = comma_at.take() {
                    out.replace_range(comma..=comma, " ");
                }
                last = Some(c);
                out.push(c);
            }
            c if c.is_ascii_whitespace() => out.push(c),
            c => {
                comma_at = None;
                last = Some(c);
                out.push(c);
            }
        }
    }
    (out, problem)
}

/// Appends what replaces `c` inside a comment: one space per byte, so offsets hold; line ends stay.
fn blank(out: &mut String, c: char) {
    if c == '\n' || c == '\r' {
        out.push(c);
    } else {
        out.extend(std::iter::repeat_n(' ', c.len_utf8()));
    }
}

/// `line L column C` (both from 1, the column counted in bytes) of a byte offset, the way
/// `serde_json` words its own positions.
fn position(text: &str, offset: usize) -> String {
    let bytes = text.as_bytes();
    let before = bytes.get(..offset).unwrap_or(bytes);
    let line = before.iter().filter(|&&byte| byte == b'\n').count() + 1;
    let line_start = before
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |newline| newline + 1);
    format!("line {line} column {}", before.len() - line_start + 1)
}

/// Checks a preload name (decision D4) and returns it lower-cased.
///
/// After ASCII lower-casing the name must be labels of `[a-z0-9-]`, 1 to 63 bytes each, joined by
/// single dots, 253 bytes at most. Chromium writes internationalised names as punycode
/// (`xn--...`), and the list Servo shipped has only `[a-z0-9.-]`; a Unicode name, `_`, `*`, an
/// empty label or a dot at either end is an error, because dropping or repairing it would change
/// security data without a person looking. The error names the entry.
pub fn valid_host(name: &str) -> Result<String, String> {
    let host = name.to_ascii_lowercase();
    if host.is_empty() {
        return refuse(name, "is empty");
    }
    for c in host.chars() {
        match c {
            'a'..='z' | '0'..='9' | '-' | '.' => {}
            '_' => return refuse(name, "contains '_', which a host name cannot have"),
            '*' => return refuse(name, "contains '*'; wildcard entries are not supported"),
            c if !c.is_ascii() => {
                return refuse(
                    name,
                    &format!(
                        "contains the non-ASCII character {c:?}; internationalised names must \
                         be written as punycode (xn--...)"
                    ),
                );
            }
            c => {
                return refuse(
                    name,
                    &format!("contains {c:?}, which is not a letter, a digit, '-' or '.'"),
                );
            }
        }
    }
    if host.starts_with('.') {
        return refuse(name, "starts with a dot");
    }
    if host.ends_with('.') {
        return refuse(name, "ends with a dot");
    }
    for label in host.split('.') {
        if label.is_empty() {
            return refuse(name, "has an empty label (two dots in a row)");
        }
        if label.len() > MAX_LABEL_BYTES {
            return refuse(
                name,
                &format!(
                    "has a label of {} bytes; the limit is {MAX_LABEL_BYTES}",
                    label.len()
                ),
            );
        }
    }
    if host.len() > MAX_HOST_BYTES {
        return refuse(
            name,
            &format!(
                "is {} bytes long; the limit is {MAX_HOST_BYTES}",
                host.len()
            ),
        );
    }
    Ok(host)
}

fn refuse<T>(name: &str, why: &str) -> Result<T, String> {
    Err(format!("host name {} {why}", shown(name)))
}

/// A name quoted for an error message, cut when it is very long.
fn shown(name: &str) -> String {
    let mut text: String = name.chars().take(SHOWN_CHARS).collect();
    if name.chars().nth(SHOWN_CHARS).is_some() {
        text.push_str("...");
    }
    format!("{text:?}")
}

/// `entries[3] ("example.com")`: how an error points at an object of the list.
fn which(index: usize, name: &str) -> String {
    format!("entries[{index}] ({})", shown(name))
}

/// What kind of JSON value this is, for an error message ("a string", "null").
fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// Reads Chromium's `transport_security_state_static.json` (already base64-decoded) into the
/// preload entries and a report of what was in it.
///
/// The text must be UTF-8 (a leading byte order mark is ignored). Comments and trailing commas are
/// accepted ([`strip_comments`]). The top level must be an object whose `"entries"` is a list of
/// objects, each with a string `"name"`. Errors say what is wrong and where: the object's index
/// and name, or the line and column of a JSON error or of the string or comment that is never
/// closed. See the module documentation for the selection rules.
pub fn parse_chromium(json: &[u8]) -> Result<Parsed, String> {
    let text = std::str::from_utf8(json).map_err(|error| {
        format!(
            "the Chromium list is not UTF-8 text (the first bad byte is at offset {})",
            error.valid_up_to()
        )
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (stripped, problem) = scan(text);
    if let Some(problem) = problem {
        return Err(format!("the Chromium list is not valid JSON: {problem}"));
    }
    let value: Value = serde_json::from_str(&stripped)
        .map_err(|error| format!("the Chromium list is not valid JSON: {error}"))?;
    let top = value.as_object().ok_or_else(|| {
        format!(
            "the top level of the Chromium list is {}, not an object",
            describe(&value)
        )
    })?;
    let list = match top.get("entries") {
        Some(Value::Array(list)) => list,
        Some(other) => {
            return Err(format!(
                "\"entries\" in the Chromium list is {}, not a list",
                describe(other)
            ));
        }
        None => {
            let keys: Vec<&str> = top.keys().take(10).map(String::as_str).collect();
            return Err(format!(
                "the Chromium list has no \"entries\" key (its top-level keys: {})",
                if keys.is_empty() {
                    "none".to_string()
                } else {
                    keys.join(", ")
                }
            ));
        }
    };

    let mut report = SourceReport {
        objects: list.len(),
        ..SourceReport::default()
    };
    let mut hosts: BTreeMap<String, bool> = BTreeMap::new();
    for (index, item) in list.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| format!("entries[{index}] is {}, not an object", describe(item)))?;
        let name = match object.get("name") {
            Some(Value::String(name)) => name.as_str(),
            Some(other) => {
                return Err(format!(
                    "entries[{index}] has a \"name\" that is {}, not a string",
                    describe(other)
                ));
            }
            None => return Err(format!("entries[{index}] has no \"name\"")),
        };
        for key in object.keys() {
            if !KNOWN_KEYS.contains(&key.as_str()) {
                *report.unknown_keys.entry(key.clone()).or_insert(0) += 1;
            }
        }
        let force_https = match object.get("mode") {
            None => false,
            Some(Value::String(mode)) => mode == MODE_FORCE_HTTPS,
            Some(other) => {
                return Err(format!(
                    "{} has a \"mode\" that is {}, not a string",
                    which(index, name),
                    describe(other)
                ));
            }
        };
        if !force_https {
            report.skipped += 1;
            continue;
        }
        report.force_https += 1;
        let include_subdomains = match object.get("include_subdomains") {
            None => false,
            Some(Value::Bool(flag)) => *flag,
            Some(other) => {
                return Err(format!(
                    "{} has an \"include_subdomains\" that is {}, not true or false",
                    which(index, name),
                    describe(other)
                ));
            }
        };
        let host = valid_host(name).map_err(|reason| format!("entries[{index}]: {reason}"))?;
        match hosts.entry(host) {
            MapEntry::Occupied(mut seen) => {
                report.merged_duplicates += 1;
                *seen.get_mut() |= include_subdomains;
            }
            MapEntry::Vacant(slot) => {
                slot.insert(include_subdomains);
            }
        }
    }
    let entries = hosts
        .into_iter()
        .map(|(host, include_subdomains)| Entry {
            host,
            include_subdomains,
        })
        .collect();
    Ok(Parsed { entries, report })
}
