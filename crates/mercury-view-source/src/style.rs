// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The style sheet of the source page: colour tokens for light and dark, and the class table of
//! the highlighter. Item 1 (source-page) writes this module.
//!
//! The colours are written once, in [`PALETTE`]. The style sheet is generated from it, so what the
//! contrast test reads is what the page carries. Rules the sheet keeps (see the brief):
//!
//! - colour tokens live on `:root`, once for light and once under `prefers-color-scheme: dark`;
//! - the page background is set on `html` and `body` (the browser's own background never shows);
//! - line numbers come from `content: attr(data-n)` in a `::before`, never from CSS counters
//!   (Servo bug ED5: `counter()` can be broken in 0.8);
//! - a grid keeps the gutter beside the first row of a wrapped line;
//! - wrapping is on by default and switched by a visually hidden checkbox that precedes `<main>`;
//!   no script is involved;
//! - nothing depends on `user-select`: generated content is not copied with the text anyway.

use std::fmt::Write as _;
use std::sync::OnceLock;

use crate::highlight::CLASSES;

/// One colour token: a name, and its value in the light and in the dark scheme.
pub(crate) struct Token {
    /// The custom property is `--<name>`.
    pub name: &'static str,
    /// `#rrggbb` for `prefers-color-scheme: light` (and when no preference is known).
    pub light: &'static str,
    /// `#rrggbb` for `prefers-color-scheme: dark`.
    pub dark: &'static str,
}

/// Every colour of the page. Text colours and the backgrounds they sit on are listed in
/// [`CONTRAST_PAIRS`]; `border` and `focus` are lines, not text, and are not in it.
pub(crate) const PALETTE: &[Token] = &[
    // Page and header.
    Token {
        name: "bg",
        light: "#ffffff",
        dark: "#0d1117",
    },
    Token {
        name: "fg",
        light: "#1f2328",
        dark: "#e6edf3",
    },
    Token {
        name: "muted",
        light: "#59636e",
        dark: "#9198a1",
    },
    Token {
        name: "surface",
        light: "#f6f8fa",
        dark: "#161b22",
    },
    Token {
        name: "border",
        light: "#d1d9e0",
        dark: "#30363d",
    },
    Token {
        name: "link",
        light: "#0550ae",
        dark: "#79c0ff",
    },
    Token {
        name: "focus",
        light: "#0969da",
        dark: "#79c0ff",
    },
    Token {
        name: "notice-bg",
        light: "#fff8c5",
        dark: "#3a2d00",
    },
    Token {
        name: "notice-fg",
        light: "#7d4e00",
        dark: "#f2cc60",
    },
    Token {
        name: "error",
        light: "#a40e26",
        dark: "#ff9492",
    },
    // Syntax: one token per class of the highlighter (see `highlight::CLASSES`).
    Token {
        name: "comment",
        light: "#4d6b3b",
        dark: "#8b949e",
    },
    Token {
        name: "keyword",
        light: "#cf222e",
        dark: "#ff7b72",
    },
    Token {
        name: "string",
        light: "#0a3069",
        dark: "#a5d6ff",
    },
    Token {
        name: "special",
        light: "#0e6f78",
        dark: "#56d4dd",
    },
    Token {
        name: "number",
        light: "#0550ae",
        dark: "#79c0ff",
    },
    Token {
        name: "function",
        light: "#8250df",
        dark: "#d2a8ff",
    },
    Token {
        name: "type",
        light: "#953800",
        dark: "#ffa657",
    },
    Token {
        name: "tag",
        light: "#116329",
        dark: "#7ee787",
    },
    Token {
        name: "attribute",
        light: "#a0116f",
        dark: "#ff8bcb",
    },
    Token {
        name: "tag-error",
        light: "#a40e26",
        dark: "#ff9492",
    },
];

#[cfg(test)]
/// `(text colour, background colour)` pairs that must reach 4.5:1 in both schemes. Every syntax
/// colour is checked on the page background and on `surface`, the colour of a hovered line.
pub(crate) const CONTRAST_PAIRS: &[(&str, &str)] = &[
    ("fg", "bg"),
    ("fg", "surface"),
    ("muted", "bg"),
    ("muted", "surface"),
    ("link", "bg"),
    ("link", "surface"),
    ("error", "bg"),
    ("error", "surface"),
    ("notice-fg", "notice-bg"),
    ("comment", "bg"),
    ("comment", "surface"),
    ("keyword", "bg"),
    ("keyword", "surface"),
    ("string", "bg"),
    ("string", "surface"),
    ("special", "bg"),
    ("special", "surface"),
    ("number", "bg"),
    ("number", "surface"),
    ("function", "bg"),
    ("function", "surface"),
    ("type", "bg"),
    ("type", "surface"),
    ("tag", "bg"),
    ("tag", "surface"),
    ("attribute", "bg"),
    ("attribute", "surface"),
    ("tag-error", "bg"),
    ("tag-error", "surface"),
];

/// The widest line number, in digits, the gutter has a class for (`MAX_SHOWN_LINES` is 50,000).
pub(crate) const MAX_GUTTER_DIGITS: usize = 6;

/// The fixed part of the sheet. Colours are always `var(--name)`; the generated part (the
/// tokens, the syntax classes and the gutter widths) is added around it.
const BODY_CSS: &str = "\
html{background:var(--bg)}\
body{margin:0;background:var(--bg);color:var(--fg);\
font:13px/1.55 ui-monospace,SFMono-Regular,\"SF Mono\",Menlo,Consolas,\"Liberation Mono\",\"DejaVu Sans Mono\",monospace}\
#wrap{position:absolute;width:1px;height:1px;margin:0;opacity:0}\
header{padding:12px 16px 10px;background:var(--surface);border-bottom:1px solid var(--border);\
font-family:system-ui,-apple-system,\"Segoe UI\",Roboto,\"Helvetica Neue\",Arial,sans-serif;font-size:13px;line-height:1.5}\
h1{margin:0 0 4px;font-size:16px;font-weight:600}\
header p{margin:4px 0}\
header a{color:var(--link)}\
.addr{overflow-wrap:break-word;word-wrap:break-word;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}\
.facts{color:var(--muted)}\
.facts span{display:inline-block;margin-right:16px}\
.note{color:var(--muted)}\
.notice{padding:6px 10px;background:var(--notice-bg);color:var(--notice-fg);border-radius:4px}\
main .notice{margin:12px 16px 0;font-family:system-ui,-apple-system,\"Segoe UI\",Roboto,Arial,sans-serif}\
.err{color:var(--error);font-weight:600}\
label{display:inline-block;margin-top:6px;color:var(--link);cursor:pointer}\
label::before{content:\"\";display:inline-block;box-sizing:border-box;width:13px;height:13px;\
margin-right:7px;border:2px solid var(--link);border-radius:3px;vertical-align:-2px}\
#wrap:checked~header label::before{background:var(--link)}\
#wrap:checked~header label::after{content:\": on\"}\
#wrap:not(:checked)~header label::after{content:\": off\"}\
#wrap:focus~header label{outline:2px solid var(--focus);outline-offset:3px}\
main{display:block;overflow-x:auto;padding:8px 0 32px}\
.src{display:block}\
.l{display:grid;grid-template-columns:var(--gw) minmax(0,1fr);\
white-space:pre-wrap;overflow-wrap:break-word;word-wrap:break-word;\
tab-size:4;-moz-tab-size:4}\
.l::before{content:attr(data-n);padding:0 1ch;text-align:right;color:var(--muted)}\
.l:hover{background:var(--surface)}\
#wrap:not(:checked)~main .l{white-space:pre;overflow-wrap:normal;word-wrap:normal;\
grid-template-columns:var(--gw) max-content;width:max-content;min-width:100%}\
.src a{color:inherit;text-decoration:underline dotted}\
.src a:hover{text-decoration:underline solid}\
.c{font-style:italic}\
.e{text-decoration:underline}";

fn push_tokens(css: &mut String, scheme: fn(&Token) -> &'static str) {
    for token in PALETTE {
        let _ = write!(css, "--{}:{};", token.name, scheme(token));
    }
}

fn build() -> String {
    let mut css = String::with_capacity(4096);
    css.push_str(":root{color-scheme:light dark;");
    push_tokens(&mut css, |token| token.light);
    css.push_str("}@media (prefers-color-scheme:dark){:root{");
    push_tokens(&mut css, |token| token.dark);
    css.push_str("}}");
    css.push_str(BODY_CSS);
    for class in CLASSES {
        let _ = write!(css, ".{}{{color:var(--{})}}", class.css, class.token);
    }
    for digits in 1..=MAX_GUTTER_DIGITS {
        // The gutter is the digits plus one character of padding on each side.
        let _ = write!(css, ".d{digits}{{--gw:{}ch}}", digits + 2);
    }
    css
}

/// The style sheet, built once.
pub(crate) fn stylesheet() -> &'static str {
    static SHEET: OnceLock<String> = OnceLock::new();
    SHEET.get_or_init(build)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value of `--name` inside the first `{...}` that follows `marker` in the sheet.
    fn hex_of(sheet: &str, marker: &str, name: &str) -> Option<[u8; 3]> {
        let from = sheet.find(marker)?;
        let block_end = sheet[from..].find('}')? + from;
        let block = &sheet[from..block_end];
        let key = format!("--{name}:#");
        let start = block.find(&key)? + key.len();
        let digits = block.get(start..start + 6)?;
        let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
        Some([channel(0)?, channel(2)?, channel(4)?])
    }

    fn linear(channel: u8) -> f64 {
        let c = f64::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// WCAG 2.x relative luminance.
    fn luminance([r, g, b]: [u8; 3]) -> f64 {
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn the_luminance_function_matches_known_ratios() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
        assert!((contrast([255, 255, 255], [255, 255, 255]) - 1.0).abs() < 1e-9);
        // WCAG's own example: #777777 on white is 4.48:1 (just under the bar).
        let grey = contrast([0x77, 0x77, 0x77], [255, 255, 255]);
        assert!((4.4..4.5).contains(&grey), "{grey}");
        // And #767676, the lightest grey that passes, is 4.54:1.
        let passes = contrast([0x76, 0x76, 0x76], [255, 255, 255]);
        assert!((4.5..4.6).contains(&passes), "{passes}");
    }

    #[test]
    fn every_text_colour_reaches_4_5_to_1_in_both_schemes() {
        let sheet = stylesheet();
        // The light tokens are the first block; the dark ones are the block inside the media query.
        let blocks = [
            ("light", ":root{"),
            ("dark", "(prefers-color-scheme:dark){:root{"),
        ];
        let mut worst = (f64::MAX, String::new());
        for (scheme, marker) in blocks {
            for (text, background) in CONTRAST_PAIRS {
                let fg = hex_of(sheet, marker, text)
                    .unwrap_or_else(|| panic!("{scheme}: --{text} is not in the sheet"));
                let bg = hex_of(sheet, marker, background)
                    .unwrap_or_else(|| panic!("{scheme}: --{background} is not in the sheet"));
                let ratio = contrast(fg, bg);
                if ratio < worst.0 {
                    worst = (ratio, format!("{scheme}: {text} on {background}"));
                }
                assert!(
                    ratio >= 4.5,
                    "{scheme}: --{text} on --{background} is {ratio:.2}:1, below 4.5:1"
                );
            }
        }
        eprintln!("lowest contrast: {:.2}:1 ({})", worst.0, worst.1);
    }

    #[test]
    fn every_syntax_class_has_a_colour_that_is_checked() {
        for class in CLASSES {
            assert!(
                PALETTE.iter().any(|token| token.name == class.token),
                "class .{} uses --{}, which is not in the palette",
                class.css,
                class.token
            );
            assert!(
                CONTRAST_PAIRS
                    .iter()
                    .any(|(text, background)| *text == class.token && *background == "bg"),
                "--{} is not checked against the page background",
                class.token
            );
            assert!(
                stylesheet().contains(&format!(".{}{{color:var(--{})}}", class.css, class.token)),
                "the sheet has no rule for .{}",
                class.css
            );
        }
    }

    #[test]
    fn every_palette_token_is_in_both_schemes_with_a_six_digit_value() {
        for token in PALETTE {
            for (scheme, value) in [("light", token.light), ("dark", token.dark)] {
                assert!(
                    value.len() == 7
                        && value.starts_with('#')
                        && value[1..].bytes().all(|b| b.is_ascii_hexdigit()),
                    "{scheme} --{}: {value:?} is not #rrggbb",
                    token.name
                );
            }
        }
        let names: std::collections::HashSet<_> = PALETTE.iter().map(|t| t.name).collect();
        assert_eq!(names.len(), PALETTE.len(), "a token name is used twice");
    }

    #[test]
    fn the_sheet_has_the_documented_shape() {
        let sheet = stylesheet();
        // Tokens on :root, light first, dark under the media query.
        assert!(sheet.starts_with(":root{color-scheme:light dark;--bg:#ffffff;"));
        assert!(sheet.contains("@media (prefers-color-scheme:dark){:root{--bg:#0d1117;"));
        // Page background on html and body.
        assert!(sheet.contains("html{background:var(--bg)}"));
        assert!(sheet.contains("body{margin:0;background:var(--bg);"));
        // Line numbers: attr(), never counters (Servo bug ED5).
        assert!(sheet.contains("content:attr(data-n)"));
        for banned in [
            "counter(",
            "counter-reset",
            "counter-increment",
            "user-select",
        ] {
            assert!(!sheet.contains(banned), "the sheet must not use {banned}");
        }
        // A grid gutter, and wrap on by default with a checkbox and sibling selectors.
        assert!(sheet.contains(".l{display:grid;grid-template-columns:var(--gw) minmax(0,1fr);"));
        assert!(sheet.contains("white-space:pre-wrap"));
        assert!(sheet.contains("#wrap:not(:checked)~main .l{white-space:pre;"));
        // Nothing that fetches, imports or could close the style element.
        for banned in [
            "@import",
            "url(",
            "expression(",
            "javascript:",
            "<",
            "@font-face",
        ] {
            assert!(
                !sheet.contains(banned),
                "the sheet must not contain {banned}"
            );
        }
        // Every gutter class the page can ask for exists.
        for digits in 1..=MAX_GUTTER_DIGITS {
            assert!(sheet.contains(&format!(".d{digits}{{--gw:{}ch}}", digits + 2)));
        }
        // Colour literals appear only in the two token blocks; the rest uses `var(--...)`.
        let rules = &sheet[sheet.find("html{background").unwrap_or(0)..];
        assert!(
            rules.replace("#wrap", "").find('#').is_none(),
            "a colour literal outside the palette"
        );
    }
}
