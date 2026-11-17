// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use mercury_passwords::Origin;

/// The longest user name or realm shown, in characters; longer ones end in an ellipsis.
const MAX_SHOWN_CHARS: usize = 120;

/// The site as the user reads it: the host and any port for HTTPS (`example.com`,
/// `example.com:8443`), the whole origin for plain-HTTP loopback sites (`http://localhost:8000`),
/// so that an unencrypted site never looks like an encrypted one. Hosts are ASCII: an
/// internationalised name is shown in its `xn--` form.
pub fn site_label(origin: &Origin) -> String {
    let text = origin.as_str();
    text.strip_prefix("https://").unwrap_or(text).to_owned()
}

/// Text read from the store, made safe to show: anything may have been written there by another
/// program (see the `mercury-passwords` developer guide). Control characters, line and paragraph
/// separators and bidirectional formatting characters become U+FFFD, so they can neither break
/// nor reorder the text around them, and long text is shortened.
pub fn display_text(text: &str) -> String {
    let mut shown = String::with_capacity(text.len().min(MAX_SHOWN_CHARS * 4));
    for (index, character) in text.chars().enumerate() {
        if index == MAX_SHOWN_CHARS {
            shown.push('…');
            break;
        }
        shown.push(if is_unsafe(character) {
            '\u{fffd}'
        } else {
            character
        });
    }
    shown
}

fn is_unsafe(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{2028}'
                | '\u{2029}'
                | '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(text: &str) -> Origin {
        Origin::parse(text).expect("a valid origin")
    }

    #[test]
    fn https_sites_show_their_host_and_port() {
        assert_eq!(site_label(&origin("https://example.com/a")), "example.com");
        assert_eq!(
            site_label(&origin("https://example.com:8443")),
            "example.com:8443"
        );
        assert_eq!(
            site_label(&origin("https://bücher.example")),
            "xn--bcher-kva.example"
        );
    }

    #[test]
    fn plain_http_sites_keep_their_scheme() {
        assert_eq!(
            site_label(&origin("http://localhost:8000/")),
            "http://localhost:8000"
        );
    }

    #[test]
    fn stored_text_cannot_break_or_reorder_the_line() {
        assert_eq!(display_text("alice"), "alice");
        assert_eq!(display_text("a\nb\tc"), "a\u{fffd}b\u{fffd}c");
        assert_eq!(
            display_text("bob\u{202e}moc.evil\u{2069}"),
            "bob\u{fffd}moc.evil\u{fffd}"
        );
        assert_eq!(display_text("x\u{2028}y"), "x\u{fffd}y");
        assert_eq!(display_text("Zoë ✓"), "Zoë ✓");
    }

    #[test]
    fn long_text_is_shortened() {
        let long = "a".repeat(500);

        let shown = display_text(&long);

        assert_eq!(shown.chars().count(), MAX_SHOWN_CHARS + 1);
        assert!(shown.ends_with('…'));
    }
}
