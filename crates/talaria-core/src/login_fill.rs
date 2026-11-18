// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Filling a saved login into the page, after the user chose it in the browser's own menu.

use std::fmt;
use std::fmt::Write as _;

/// The script that fills; see the comments in the file for its checks.
const FILL_SCRIPT: &str = include_str!("login_fill.js");

/// A login to fill into the focused password field of a tab's top-level document. `Debug` shows
/// the origin only.
#[derive(Clone, PartialEq, Eq)]
pub struct LoginFill {
    origin: String,
    username: String,
    password: String,
}

impl LoginFill {
    /// `origin` is the serialised origin the top-level document must have
    /// (`https://example.com`); the script fills nothing anywhere else.
    pub const fn new(origin: String, username: String, password: String) -> Self {
        Self {
            origin,
            username,
            password,
        }
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The script for the engine to evaluate in the top-level document. It returns a status:
    /// `filled`, `filled-password` (no user-name field found, or no user name), `wrong-origin`,
    /// `no-password-field` or `read-only`. The script holds the password: never log it.
    pub fn script(&self) -> String {
        let mut script = String::with_capacity(
            FILL_SCRIPT.len() + self.origin.len() + self.username.len() + self.password.len() + 16,
        );
        script.push_str(FILL_SCRIPT.trim_end());
        script.push('(');
        push_js_string(&mut script, &self.origin);
        script.push_str(", ");
        push_js_string(&mut script, &self.username);
        script.push_str(", ");
        push_js_string(&mut script, &self.password);
        script.push_str(");");
        script
    }
}

impl fmt::Debug for LoginFill {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginFill")
            .field("origin", &self.origin)
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Appends `text` as a double-quoted JavaScript string literal. Quotes, backslashes, control
/// characters, `<` and the line and paragraph separators are escaped, so no value can end the
/// literal or the script.
fn push_js_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' | '>' | '\u{2028}' | '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(character));
            }
            character if character.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(character));
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(text: &str) -> String {
        let mut out = String::new();
        push_js_string(&mut out, text);
        out
    }

    #[test]
    fn values_cannot_end_the_string_or_the_script() {
        assert_eq!(literal("plain"), "\"plain\"");
        assert_eq!(literal("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(literal("x\ny\r\u{0}"), "\"x\\u000ay\\u000d\\u0000\"");
        assert_eq!(literal("</script>"), "\"\\u003c/script\\u003e\"");
        assert_eq!(literal("p\u{2028}q\u{2029}"), "\"p\\u2028q\\u2029\"");
        assert_eq!(literal("Zoë ✓"), "\"Zoë ✓\"");
    }

    #[test]
    fn the_script_calls_the_fill_function_with_the_three_values() {
        let fill = LoginFill::new(
            "https://example.com".to_owned(),
            "alice".to_owned(),
            "fake\"pw".to_owned(),
        );

        let script = fill.script();

        assert!(script.contains("// Fills a saved login"));
        assert!(script.ends_with(r#"})("https://example.com", "alice", "fake\"pw");"#));
        assert!(script.contains("location.origin !== origin"));
        assert!(script.contains("document.activeElement"));
        // Hidden inputs (no layout box, such as honeypots) are never the user-name partner.
        assert!(script.contains("getClientRects().length > 0"));
    }

    #[test]
    fn debug_output_shows_only_the_origin() {
        let fill = LoginFill::new(
            "https://example.com".to_owned(),
            "alice".to_owned(),
            "fake-secret".to_owned(),
        );

        let shown = format!("{fill:?}");

        assert!(shown.contains("https://example.com"));
        assert!(!shown.contains("alice"));
        assert!(!shown.contains("fake-secret"));
    }
}
