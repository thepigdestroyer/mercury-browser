/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: keeps cookie values and credentials out of the logs (EC13).
//!
//! Servo's HTTP loader logs every request and response header at `debug` level, and the cookie jar logged
//! the cookie string it sent at `info` level. Both carry session cookies and credentials. The functions
//! here replace the value of `Cookie`, `Set-Cookie`, `Authorization` and `Proxy-Authorization` with
//! [`REDACTED`] and leave every other header as it was, so a debug log still shows what was sent.
//!
//! This is a Mercury file, not an upstream one. Drop it (with its two call sites in `http_loader.rs`, the
//! two log lines in `cookie.rs` and `cookie_storage.rs`, and the `mod mercury_log;` line in `lib.rs`) at the
//! Servo release that no longer logs these values.

use http::header::{HeaderMap, HeaderName, HeaderValue};

/// What the value of a secret header is replaced with.
pub(crate) const REDACTED: &str = "<redacted>";

/// Headers whose value is a credential. Compared without regard to case.
const SECRET_HEADERS: [&str; 4] = [
    "cookie",
    "set-cookie",
    "authorization",
    "proxy-authorization",
];

fn is_secret(name: &HeaderName) -> bool {
    SECRET_HEADERS
        .iter()
        .any(|secret| name.as_str().eq_ignore_ascii_case(secret))
}

/// The value as the log shows it: quoted like `HeaderValue`'s own `Debug`, or [`REDACTED`] for a secret.
fn value_for_log(name: &HeaderName, value: &HeaderValue) -> String {
    if is_secret(name) {
        format!("{REDACTED:?}")
    } else {
        format!("{value:?}")
    }
}

/// One header as a log line shows it: `("name", "value")`, the same shape as `{:?}` of the pair that
/// `HeaderMap::iter` yields, with the value of a secret header redacted.
pub(crate) fn header_for_log(name: &HeaderName, value: &HeaderValue) -> String {
    format!("({name:?}, {})", value_for_log(name, value))
}

/// A whole header map as a log line shows it: `{"name": "value", ...}`, the same shape as `{:?}` of a
/// `HeaderMap`, with the values of secret headers redacted.
pub(crate) fn headers_for_log(headers: &HeaderMap) -> String {
    let entries: Vec<String> = headers
        .iter()
        .map(|(name, value)| format!("{name:?}: {}", value_for_log(name, value)))
        .collect();
    format!("{{{}}}", entries.join(", "))
}

#[cfg(test)]
mod tests {
    use http::header::{ACCEPT, AUTHORIZATION, COOKIE, PROXY_AUTHORIZATION, SET_COOKIE};

    use super::*;

    const SECRET: &str = "sessionid=s3cr3t-value-1234";

    fn value(text: &str) -> HeaderValue {
        HeaderValue::from_str(text).unwrap()
    }

    #[test]
    fn the_value_of_each_secret_header_is_redacted() {
        for name in [COOKIE, SET_COOKIE, AUTHORIZATION, PROXY_AUTHORIZATION] {
            let line = header_for_log(&name, &value(SECRET));
            assert!(!line.contains("s3cr3t"), "{name}: {line}");
            assert!(line.contains(REDACTED), "{name}: {line}");
            // The name stays, so the log still says which header was sent.
            assert!(line.contains(name.as_str()), "{name}: {line}");
        }
    }

    #[test]
    fn the_names_are_matched_without_regard_to_case() {
        for spelled in [
            "Cookie",
            "COOKIE",
            "cOoKiE",
            "Set-Cookie",
            "AUTHORIZATION",
            "Proxy-Authorization",
        ] {
            let name = HeaderName::from_bytes(spelled.as_bytes()).unwrap();
            let line = header_for_log(&name, &value(SECRET));
            assert!(!line.contains("s3cr3t"), "{spelled}: {line}");
            assert!(line.contains(REDACTED), "{spelled}: {line}");
        }
    }

    #[test]
    fn other_headers_are_printed_as_before() {
        let accept = value("text/html,application/xhtml+xml");
        // The shape of the old `debug!(" - {:?}", header)`: a `(name, value)` pair.
        assert_eq!(
            header_for_log(&ACCEPT, &accept),
            format!("{:?}", (&ACCEPT, &accept))
        );
        assert_eq!(
            header_for_log(&ACCEPT, &accept),
            r#"("accept", "text/html,application/xhtml+xml")"#
        );
    }

    #[test]
    fn an_empty_secret_value_is_redacted_too() {
        // Not telling a log reader whether the credential was empty costs nothing.
        let line = header_for_log(&AUTHORIZATION, &value(""));
        assert_eq!(line, format!("(\"authorization\", {REDACTED:?})"));
    }

    #[test]
    fn a_header_map_keeps_every_entry_and_hides_the_secrets() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, value("*/*"));
        headers.insert(COOKIE, value(SECRET));
        headers.insert(AUTHORIZATION, value("Bearer t0ken-abcdef"));
        headers.append(SET_COOKIE, value("a=first-secret; Path=/"));
        headers.append(SET_COOKIE, value("b=second-secret; Secure"));

        let line = headers_for_log(&headers);
        for leaked in ["s3cr3t", "t0ken", "first-secret", "second-secret"] {
            assert!(!line.contains(leaked), "{leaked} leaked: {line}");
        }
        assert!(line.starts_with('{') && line.ends_with('}'), "{line}");
        assert!(line.contains(r#""accept": "*/*""#), "{line}");
        // Two Set-Cookie headers are two entries, as in `HeaderMap::iter`.
        assert_eq!(
            line.matches(r#""set-cookie": "<redacted>""#).count(),
            2,
            "{line}"
        );
        assert!(line.contains(r#""cookie": "<redacted>""#), "{line}");
        assert!(line.contains(r#""authorization": "<redacted>""#), "{line}");
    }

    #[test]
    fn an_empty_header_map_prints_empty_braces() {
        assert_eq!(headers_for_log(&HeaderMap::new()), "{}");
    }

    // ---- The guard: nobody may log a cookie value again --------------------------------------------

    /// Every logging macro call in `source`, as the text between its parentheses.
    fn log_macro_calls(source: &str) -> Vec<&str> {
        const MACROS: [&str; 8] = [
            "info", "debug", "trace", "warn", "error", "println", "eprintln", "dbg",
        ];
        let bytes = source.as_bytes();
        let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let mut calls = Vec::new();
        for name in MACROS {
            let needle = format!("{name}!(");
            let mut from = 0;
            while let Some(found) = source[from..].find(&needle) {
                let start = from + found;
                from = start + needle.len();
                if start > 0 && is_ident(bytes[start - 1]) {
                    continue; // `some_info!(`, not the log macro
                }
                if let Some(end) = matching_paren(source, from) {
                    calls.push(&source[from..end]);
                }
            }
        }
        calls
    }

    /// The index of the `)` that closes a `(` opened just before `from`, skipping string and char literals.
    fn matching_paren(source: &str, from: usize) -> Option<usize> {
        let bytes = source.as_bytes();
        let mut depth = 1usize;
        let mut i = from;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => i = end_of_string(bytes, i)?,
                b'\'' => i += char_literal_len(bytes, i) - 1,
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                },
                _ => {},
            }
            i += 1;
        }
        None
    }

    /// The index of the closing quote of the string literal that opens at `open`.
    fn end_of_string(bytes: &[u8], open: usize) -> Option<usize> {
        let mut i = open + 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 1,
                b'"' => return Some(i),
                _ => {},
            }
            i += 1;
        }
        None
    }

    /// How many bytes a `'` at `at` takes: a char literal (`'x'`, `'\n'`), or 1 for a lifetime.
    fn char_literal_len(bytes: &[u8], at: usize) -> usize {
        match (bytes.get(at + 1), bytes.get(at + 2), bytes.get(at + 3)) {
            (Some(b'\\'), _, Some(b'\'')) => 4,
            (Some(_), Some(b'\''), _) => 3,
            _ => 1,
        }
    }

    /// The comma-separated arguments of a macro call, trimmed; commas inside brackets and literals do not split.
    fn arguments(call: &str) -> Vec<&str> {
        let bytes = call.as_bytes();
        let mut parts = Vec::new();
        let (mut depth, mut start, mut i) = (0usize, 0usize, 0usize);
        while i < bytes.len() {
            match bytes[i] {
                b'"' => match end_of_string(bytes, i) {
                    Some(end) => i = end,
                    None => break,
                },
                b'\'' => i += char_literal_len(bytes, i) - 1,
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b',' if depth == 0 => {
                    parts.push(call[start..i].trim());
                    start = i + 1;
                },
                _ => {},
            }
            i += 1;
        }
        parts.push(call[start..].trim());
        parts.retain(|part| !part.is_empty());
        parts
    }

    /// Names captured inline by a format string, such as `{value}` or `{value:?}`.
    fn inline_captures(literal: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut chars = literal.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c != '{' {
                continue;
            }
            if chars.peek().map(|&(_, next)| next) == Some('{') {
                chars.next(); // `{{` is a literal brace
                continue;
            }
            let name: String = literal[i + 1..]
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect();
            if !name.is_empty() {
                names.push(name);
            }
        }
        names
    }

    /// Problems with the logging calls in a cookie file: every argument must be one of `allowed`, and no
    /// format string may capture a variable by name (that would hide a value from this check).
    fn cookie_log_problems(file: &str, source: &str, allowed: &[&str]) -> Vec<String> {
        let mut problems = Vec::new();
        for call in log_macro_calls(source) {
            let args = arguments(call);
            let Some((literal, rest)) = args.split_first() else {
                continue;
            };
            for name in inline_captures(literal) {
                problems.push(format!(
                    "{file}: `{{{name}}}` in `{literal}` captures a variable"
                ));
            }
            for arg in rest {
                if !allowed.contains(arg) {
                    problems.push(format!("{file}: logs `{arg}` in `{literal}`"));
                }
            }
        }
        problems
    }

    /// Problems with the logging calls in `http_loader.rs`: a header or a header map may only be logged
    /// through `header_for_log` or `headers_for_log`.
    fn header_log_problems(file: &str, source: &str) -> Vec<String> {
        let mut problems = Vec::new();
        for call in log_macro_calls(source) {
            let args = arguments(call);
            let Some((literal, rest)) = args.split_first() else {
                continue;
            };
            for name in inline_captures(literal) {
                if name == "header" || name == "headers" {
                    problems.push(format!(
                        "{file}: `{{{name}}}` in `{literal}` captures headers"
                    ));
                }
            }
            for arg in rest {
                let mentions_headers = arg
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|word| word == "header" || word == "headers");
                let redacted =
                    arg.starts_with("header_for_log(") || arg.starts_with("headers_for_log(");
                if mentions_headers && !redacted {
                    problems.push(format!(
                        "{file}: logs `{arg}` in `{literal}` without redacting"
                    ));
                }
            }
        }
        problems
    }

    /// The only things the cookie jar may log: a cookie's name, domain and path, and how many were sent.
    const ALLOWED_COOKIE_LOG_ARGS: [&str; 4] = [
        "self.cookie.name()",
        "self.cookie.domain()",
        "self.cookie.path()",
        "sent_count",
    ];

    #[test]
    fn the_cookie_files_do_not_log_a_cookie_value() {
        for (file, source) in [
            ("cookie_storage.rs", include_str!("cookie_storage.rs")),
            ("cookie.rs", include_str!("cookie.rs")),
        ] {
            assert!(
                !log_macro_calls(source).is_empty(),
                "{file}: the guard found no logging call; it is looking at the wrong text"
            );
            let problems = cookie_log_problems(file, source, &ALLOWED_COOKIE_LOG_ARGS);
            assert!(
                problems.is_empty(),
                "a cookie value or the cookie string would reach the log (EC13); if a new log line is \
                 safe, add its arguments to ALLOWED_COOKIE_LOG_ARGS:\n{}",
                problems.join("\n")
            );
        }
    }

    #[test]
    fn the_http_loader_logs_headers_only_through_the_redacting_helpers() {
        let source = include_str!("http_loader.rs");
        let problems = header_log_problems("http_loader.rs", source);
        assert!(
            problems.is_empty(),
            "a header value would reach the log unredacted (EC13):\n{}",
            problems.join("\n")
        );
        // The guard is alive: the two header dumps and the CORS line really use the helpers.
        assert!(
            source.matches("header_for_log(").count() >= 2,
            "the two header-dump loops no longer use header_for_log"
        );
        assert!(
            source.contains("headers_for_log("),
            "the CORS debug line no longer uses headers_for_log"
        );
    }

    #[test]
    fn the_guard_finds_the_lines_servo_used_to_have() {
        // The upstream 0.8 lines, verbatim. If the guard did not flag these it would guard nothing.
        let old_storage = r#"info!(" === COOKIES SENT: {}", result);"#;
        let old_cookie = r#"debug!(
            " === SENT COOKIE : {} {} {:?} {:?}",
            self.cookie.name(),
            self.cookie.value(),
            self.cookie.domain(),
            self.cookie.path()
        );"#;
        let captured = r#"info!("sent {result}");"#;
        let problems = cookie_log_problems("old", old_storage, &ALLOWED_COOKIE_LOG_ARGS);
        assert_eq!(problems.len(), 1, "{problems:?}");
        let problems = cookie_log_problems("old", old_cookie, &ALLOWED_COOKIE_LOG_ARGS);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("self.cookie.value()"), "{problems:?}");
        let problems = cookie_log_problems("old", captured, &ALLOWED_COOKIE_LOG_ARGS);
        assert_eq!(problems.len(), 1, "{problems:?}");

        let old_dump = r#"for header in request.headers.iter() { debug!(" - {:?}", header); }"#;
        let old_cors =
            r#"debug!("CORS check: {:?}, current headers: {:?}", header_names, request.headers);"#;
        assert_eq!(header_log_problems("old", old_dump).len(), 1);
        assert_eq!(header_log_problems("old", old_cors).len(), 1);
        let redacted = r#"debug!(" - {}", header_for_log(name, value)); debug!("{}", headers_for_log(&request.headers));"#;
        assert!(header_log_problems("new", redacted).is_empty());
    }

    #[test]
    fn the_guard_reads_literals_and_nesting_correctly() {
        let source = r#"
            // not a call: some_info!(self.cookie.value())
            debug!("a, b ( ) {}", f(x, y), 'c');
            warn!("quote \" and paren ) in text, {}", self.cookie.name());
        "#;
        let calls = log_macro_calls(source);
        assert_eq!(calls.len(), 2, "{calls:?}");
        let args = arguments(calls.iter().find(|c| c.starts_with("\"a, b")).unwrap());
        assert_eq!(args, [r#""a, b ( ) {}""#, "f(x, y)", "'c'"]);
        assert_eq!(inline_captures("{{x}} {} {:?} {y} {z:?}"), ["y", "z"]);
    }
}
