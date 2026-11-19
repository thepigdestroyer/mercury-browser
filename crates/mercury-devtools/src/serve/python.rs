// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Python-compatible text helpers (`int()`, `str.strip()`, `repr()`, `html.escape()` and
//! ISO-8859-1 decoding), so that request parsing and messages stay identical to the Python
//! server's.

use std::num::IntErrorKind;

/// Python's `int()` of a string, saturating where Python's integers would simply be large.
pub(super) fn python_int(text: &str) -> Option<i64> {
    match text.trim().parse::<i64>() {
        Ok(value) => Some(value),
        Err(error) => match error.kind() {
            IntErrorKind::PosOverflow => Some(i64::MAX),
            IntErrorKind::NegOverflow => Some(i64::MIN),
            _ => None,
        },
    }
}

/// ISO-8859-1 decoding: every byte is the character with the same code.
pub(super) fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// Python's `str.isspace()` for the characters ISO-8859-1 can hold.
pub(super) fn is_python_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{1c}'..='\u{1f}' | ' ' | '\u{85}' | '\u{a0}'
    )
}

/// `str.strip()` without arguments.
pub(super) fn python_strip(text: &str) -> &str {
    text.trim_matches(is_python_space)
}

/// Python's `repr()` of a string decoded from ISO-8859-1.
pub(super) fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c < ' ' || ('\u{7f}'..='\u{a0}').contains(&c) || c == '\u{ad}' => {
                out.push_str(&format!("\\x{:02x}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `html.escape(text, quote=False)`.
pub(super) fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_like_python() {
        assert_eq!(python_repr("HTTP/x"), "'HTTP/x'");
        assert_eq!(python_repr("it's"), "\"it's\"");
        assert_eq!(python_repr("a'\""), "'a\\'\"'");
        assert_eq!(python_repr("\u{1}\u{e9}\\"), "'\\x01\u{e9}\\\\'");
    }
}
