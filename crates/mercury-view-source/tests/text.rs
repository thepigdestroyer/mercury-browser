// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `classify` and `decode`: what a response is, from its type and its first bytes, and which
//! characters its bytes are, from the header, a byte order mark or a `<meta>` declaration.

use mercury_view_source::{SourceKind, classify, decode};

use SourceKind::{Binary, Css, Html, JavaScript, Plain, Xml};

// ---- classify ----

#[test]
fn the_type_names_the_kind() {
    let table: &[(&str, SourceKind)] = &[
        ("text/html", Html),
        ("text/html; charset=utf-8", Html),
        ("TEXT/HTML", Html),
        ("  text/html  ;charset=utf-8", Html),
        ("application/xhtml+xml", Html),
        ("text/javascript", JavaScript),
        ("text/javascript; charset=utf-8", JavaScript),
        ("application/javascript", JavaScript),
        ("application/x-javascript", JavaScript),
        ("application/ecmascript", JavaScript),
        ("text/ecmascript", JavaScript),
        ("text/jscript", JavaScript),
        ("text/javascript1.5", JavaScript),
        ("text/css", Css),
        ("text/css; charset=utf-8", Css),
        ("application/xml", Xml),
        ("text/xml", Xml),
        ("image/svg+xml", Xml),
        ("application/atom+xml", Xml),
        ("application/rss+xml", Xml),
        ("application/xslt+xml", Xml),
        ("text/plain", Plain),
        ("text/plain; charset=iso-8859-1", Plain),
        ("text/csv", Plain),
        ("text/markdown", Plain),
        ("text/x-something-new", Plain),
        ("application/json", Plain),
        ("application/ld+json", Plain),
        ("application/vnd.api+json", Plain),
        ("image/png", Binary),
        ("image/jpeg", Binary),
        ("image/gif", Binary),
        ("image/webp", Binary),
        ("image/x-icon", Binary),
        ("IMAGE/PNG; foo=bar", Binary),
        ("audio/mpeg", Binary),
        ("video/mp4", Binary),
        ("font/woff2", Binary),
        ("font/ttf", Binary),
        ("application/font-woff", Binary),
        ("application/x-font-ttf", Binary),
        ("application/vnd.ms-fontobject", Binary),
        ("application/pdf", Binary),
        ("application/octet-stream", Binary),
        ("application/zip", Binary),
        ("application/gzip", Binary),
        ("application/wasm", Binary),
    ];
    for &(content_type, expected) in table {
        assert_eq!(
            classify(Some(content_type), b"<html>plain text"),
            expected,
            "{content_type:?}"
        );
        // The bytes are not asked when the type decides.
        assert_eq!(
            classify(Some(content_type), b""),
            expected,
            "{content_type:?}"
        );
    }
}

#[test]
fn a_known_type_is_not_overruled_by_what_the_bytes_look_like() {
    assert_eq!(classify(Some("text/plain"), b"<!doctype html><p>x"), Plain);
    assert_eq!(classify(Some("text/css"), b"<html>"), Css);
    assert_eq!(classify(Some("application/json"), b"<html>"), Plain);
    assert_eq!(classify(Some("image/png"), b"<html>"), Binary);
}

#[test]
fn an_unknown_or_missing_type_is_html_only_when_it_starts_with_markup() {
    for content_type in [
        None,
        Some(""),
        Some("garbage"),
        Some("application/unknown"),
        Some("application/x-custom; v=1"),
        Some("multipart/mixed"),
    ] {
        for (bytes, expected) in [
            (&b"<html><body>"[..], Html),
            (b"<!doctype html>", Html),
            (b"  \r\n\t<p>x", Html),
            (b"\xEF\xBB\xBF<html>", Html),
            (b"<?xml version=\"1.0\"?><a/>", Html),
            (b"hello <b>", Plain),
            (b"{\"a\": 1}", Plain),
            (b"", Plain),
            (b"   ", Plain),
        ] {
            assert_eq!(
                classify(content_type, bytes),
                expected,
                "{content_type:?} {bytes:?}"
            );
        }
    }
}

#[test]
fn a_nul_byte_in_the_first_kilobyte_makes_anything_binary() {
    for content_type in [
        None,
        Some("text/html"),
        Some("text/plain"),
        Some("text/css"),
        Some("text/javascript"),
        Some("application/json"),
        Some("application/xml"),
        Some("application/unknown"),
    ] {
        for at in [0, 1, 500, 1022, 1023] {
            let mut bytes = vec![b'a'; 1500];
            bytes[at] = 0;
            assert_eq!(
                classify(content_type, &bytes),
                Binary,
                "{content_type:?} at {at}"
            );
        }
        // Past the first kilobyte nothing is looked at.
        for at in [1024, 1025, 1499] {
            let mut bytes = vec![b'a'; 1500];
            bytes[at] = 0;
            assert_ne!(
                classify(content_type, &bytes),
                Binary,
                "{content_type:?} at {at}"
            );
        }
    }
    let mut far = vec![b'a'; 100_000];
    far[99_999] = 0;
    assert_eq!(classify(Some("text/plain"), &far), Plain);
}

#[test]
fn utf16_text_is_full_of_nul_bytes_and_is_not_binary() {
    let le = b"\xFF\xFE<\0p\0>\0h\0i\0";
    let be = b"\xFE\xFF\0<\0p\0>\0h\0i";
    let bare = b"<\0p\0>\0h\0i\0";
    assert_eq!(classify(Some("text/html"), le), Html);
    assert_eq!(classify(Some("text/html"), be), Html);
    assert_eq!(classify(Some("text/plain"), le), Plain);
    assert_eq!(classify(None, le), Plain);
    // Declared by the header instead of a mark.
    assert_eq!(classify(Some("text/html; charset=utf-16le"), bare), Html);
    assert_eq!(classify(Some("text/html; charset=UTF-16BE"), be), Html);
    assert_eq!(classify(Some("text/plain; charset=utf-16"), bare), Plain);
    // A UTF-8 mark or another charset is no excuse for a NUL byte.
    assert_eq!(
        classify(Some("text/html"), b"\xEF\xBB\xBF<p>\0</p>"),
        Binary
    );
    assert_eq!(classify(Some("text/html; charset=utf-8"), bare), Binary);
    assert_eq!(
        classify(Some("text/html; charset=windows-1252"), bare),
        Binary
    );
    // An image is an image whatever it is labelled inside.
    assert_eq!(classify(Some("image/png; charset=utf-16"), le), Binary);
}

// ---- decode ----

fn decoded(bytes: &[u8], content_type: Option<&str>) -> (String, &'static str) {
    decode(bytes, content_type)
}

/// "Привет" in windows-1251.
const PRIVET_1251: &[u8] = b"\xCF\xF0\xE8\xE2\xE5\xF2";
const PRIVET: &str = "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}";

#[test]
fn without_any_declaration_the_text_is_utf8_and_lossy() {
    assert_eq!(decoded(b"", None), (String::new(), "UTF-8"));
    assert_eq!(
        decoded(b"plain ascii", None),
        ("plain ascii".to_owned(), "UTF-8")
    );
    assert_eq!(
        decoded("caf\u{e9} \u{65e5}\u{672c}".as_bytes(), None),
        ("caf\u{e9} \u{65e5}\u{672c}".to_owned(), "UTF-8")
    );
    // Invalid bytes become U+FFFD, one for each bad sequence.
    assert_eq!(decoded(b"a\xFFb", None), ("a\u{fffd}b".to_owned(), "UTF-8"));
    assert_eq!(
        decoded(b"caf\xE9!", None),
        ("caf\u{fffd}!".to_owned(), "UTF-8")
    );
    // A character cut short at the end.
    assert_eq!(
        decoded(b"abc\xE2\x82", None),
        ("abc\u{fffd}".to_owned(), "UTF-8")
    );
    // A NUL byte is kept as it is.
    assert_eq!(decoded(b"a\0b", None), ("a\0b".to_owned(), "UTF-8"));
}

#[test]
fn the_charset_of_the_header_is_used() {
    for (content_type, bytes, expected, name) in [
        (
            "text/html; charset=windows-1252",
            &b"caf\xE9"[..],
            "caf\u{e9}",
            "windows-1252",
        ),
        // ISO-8859-1 is windows-1252, as the web has it.
        (
            "text/html; charset=ISO-8859-1",
            b"caf\xE9",
            "caf\u{e9}",
            "windows-1252",
        ),
        (
            "text/html; charset=latin1",
            b"caf\xE9",
            "caf\u{e9}",
            "windows-1252",
        ),
        (
            "text/plain; charset=shift_jis",
            b"\x93\xFA\x96\x7B\x8C\xEA",
            "\u{65e5}\u{672c}\u{8a9e}",
            "Shift_JIS",
        ),
        (
            "text/plain; charset=Shift_JIS",
            b"\x93\xFA\x96\x7B\x8C\xEA",
            "\u{65e5}\u{672c}\u{8a9e}",
            "Shift_JIS",
        ),
        (
            "text/plain; charset=euc-jp",
            b"\xC6\xFC\xCB\xDC\xB8\xEC",
            "\u{65e5}\u{672c}\u{8a9e}",
            "EUC-JP",
        ),
        (
            "text/plain; charset=windows-1251",
            PRIVET_1251,
            PRIVET,
            "windows-1251",
        ),
        (
            "text/plain; charset=\"windows-1251\"",
            PRIVET_1251,
            PRIVET,
            "windows-1251",
        ),
        (
            "text/plain; CHARSET = 'Windows-1251'",
            PRIVET_1251,
            PRIVET,
            "windows-1251",
        ),
        (
            "text/plain; format=flowed; charset=windows-1251; x=y",
            PRIVET_1251,
            PRIVET,
            "windows-1251",
        ),
        (
            "text/plain; charset=utf-8",
            "caf\u{e9}".as_bytes(),
            "caf\u{e9}",
            "UTF-8",
        ),
        (
            "text/plain; charset=UTF8",
            "caf\u{e9}".as_bytes(),
            "caf\u{e9}",
            "UTF-8",
        ),
        // The header says UTF-8 and the bytes are not: lossy, not another guess.
        (
            "text/plain; charset=utf-8",
            b"caf\xE9",
            "caf\u{fffd}",
            "UTF-8",
        ),
    ] {
        assert_eq!(
            decoded(bytes, Some(content_type)),
            (expected.to_owned(), name),
            "{content_type:?}"
        );
    }
}

#[test]
fn the_header_wins_over_a_meta_declaration() {
    let page = b"<meta charset=windows-1251>\xCF\xF0";
    assert_eq!(
        decoded(page, Some("text/html; charset=windows-1252")),
        (
            "<meta charset=windows-1251>\u{cf}\u{f0}".to_owned(),
            "windows-1252"
        )
    );
}

#[test]
fn a_charset_nobody_knows_is_ignored() {
    for content_type in [
        "text/html; charset=nonsense",
        "text/html; charset=utf-7",
        "text/html; charset=",
        "text/html; charset=\"\"",
        "text/html; charset=x-unknown; foo",
        // Browsers show these as one replacement character; the source is shown as text.
        "text/html; charset=iso-2022-kr",
        "text/html; charset=hz-gb-2312",
        "text/html",
    ] {
        assert_eq!(
            decoded(b"<p>caf\xC3\xA9</p>", Some(content_type)),
            ("<p>caf\u{e9}</p>".to_owned(), "UTF-8"),
            "{content_type:?}"
        );
    }
    // An unknown header charset leaves room for the meta declaration.
    assert_eq!(
        decoded(
            b"<meta charset=windows-1251>\xCF",
            Some("text/html; charset=nonsense")
        ),
        (
            "<meta charset=windows-1251>\u{41f}".to_owned(),
            "windows-1251"
        )
    );
}

#[test]
fn a_byte_order_mark_names_the_encoding_and_is_removed() {
    let utf8 = b"\xEF\xBB\xBFhi \xC3\xA9";
    assert_eq!(decoded(utf8, None), ("hi \u{e9}".to_owned(), "UTF-8"));
    assert_eq!(
        decoded(utf8, Some("text/html; charset=utf-8")),
        ("hi \u{e9}".to_owned(), "UTF-8")
    );
    let le = b"\xFF\xFEh\0i\0\xE9\0";
    assert_eq!(decoded(le, None), ("hi\u{e9}".to_owned(), "UTF-16LE"));
    let be = b"\xFE\xFF\0h\0i\0\xE9";
    assert_eq!(decoded(be, None), ("hi\u{e9}".to_owned(), "UTF-16BE"));
    // As a browser does, a mark wins over a header that says something else.
    assert_eq!(
        decoded(utf8, Some("text/html; charset=windows-1252")),
        ("hi \u{e9}".to_owned(), "UTF-8")
    );
    assert_eq!(
        decoded(le, Some("text/html; charset=utf-8")),
        ("hi\u{e9}".to_owned(), "UTF-16LE")
    );
    // And over a meta declaration.
    assert_eq!(
        decoded(b"\xEF\xBB\xBF<meta charset=windows-1251>\xC3\xA9", None),
        ("<meta charset=windows-1251>\u{e9}".to_owned(), "UTF-8")
    );
}

#[test]
fn utf16_without_a_mark_needs_the_header() {
    assert_eq!(
        decoded(b"h\0i\0", Some("text/plain; charset=utf-16le")),
        ("hi".to_owned(), "UTF-16LE")
    );
    assert_eq!(
        decoded(b"\0h\0i", Some("text/plain; charset=utf-16be")),
        ("hi".to_owned(), "UTF-16BE")
    );
    // An odd byte at the end is lossy, not a failure.
    assert_eq!(
        decoded(b"h\0i", Some("text/plain; charset=utf-16le")),
        ("h\u{fffd}".to_owned(), "UTF-16LE")
    );
}

#[test]
fn a_meta_declaration_in_the_first_kilobyte_is_used() {
    let in_1251 = |head: &str| {
        let mut bytes = head.as_bytes().to_vec();
        bytes.extend_from_slice(PRIVET_1251);
        bytes
    };
    for head in [
        "<meta charset=windows-1251>",
        "<meta charset=\"windows-1251\">",
        "<meta charset='windows-1251'>",
        "<meta charset = windows-1251 />",
        "<META CHARSET=\"Windows-1251\">",
        "<meta\tcharset=windows-1251>",
        "<meta\ncharset=windows-1251>",
        "<meta/charset=windows-1251>",
        "<html><head><title>x</title><meta charset=windows-1251></head><body>",
        "<!doctype html>\n<html lang=ru>\n<head>\n  <meta charset=windows-1251>\n",
        "<meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1251\">",
        "<meta http-equiv=\"content-type\" content=\"text/html;charset=windows-1251\">",
        "<meta content=\"text/html; charset=windows-1251\" http-equiv=\"content-type\">",
        "<meta http-equiv='Content-Type' content='text/html; charset = \"windows-1251\"'>",
        "<meta http-equiv=content-type content=\"charset=windows-1251\">",
        // A first meta that says nothing useful does not hide the second.
        "<meta name=viewport content=width><meta charset=windows-1251>",
        "<meta charset=nonsense><meta charset=windows-1251>",
        "<!-- <meta charset=utf-8> --><meta charset=windows-1251>",
    ] {
        let (text, name) = decoded(&in_1251(head), Some("text/html"));
        assert_eq!(name, "windows-1251", "{head:?}");
        assert!(text.ends_with(PRIVET), "{head:?} gave {text:?}");
    }
}

#[test]
fn a_meta_declaration_that_does_not_count_is_not_used() {
    let in_1251 = |head: &str, pad: usize| {
        let mut bytes = vec![b' '; pad];
        bytes.splice(0..0, head.bytes());
        bytes.extend_from_slice(PRIVET_1251);
        bytes
    };
    // After the first kilobyte.
    let mut late = b"<html><head>".to_vec();
    late.extend(std::iter::repeat_n(b' ', 1024));
    late.extend_from_slice(b"<meta charset=windows-1251>");
    late.extend_from_slice(PRIVET_1251);
    assert_eq!(decoded(&late, None).1, "UTF-8");
    // Cut by the end of the first kilobyte.
    let mut cut = vec![b' '; 1010];
    cut.extend_from_slice(b"<meta charset=windows-1251>");
    assert_eq!(decoded(&cut, None).1, "UTF-8");
    // Not a meta tag, or not about the character set, or inside a comment.
    for head in [
        "<metadata charset=windows-1251>",
        "<meta name=\"viewport\" content=\"charset=windows-1251\">",
        "<meta http-equiv=\"refresh\" content=\"0; charset=windows-1251\">",
        "<meta http-equiv=\"content-type\" content=\"text/html\">",
        "<meta http-equiv=\"content-type\">",
        "<meta charset=>",
        "<meta charset=\"\">",
        "<meta charset=nonsense>",
        "<!-- <meta charset=windows-1251> -->",
        "<!-- <meta charset=windows-1251>",
        "<p>meta charset=windows-1251</p>",
        "<meta charset=windows-1251",
    ] {
        assert_eq!(
            decoded(&in_1251(head, 0), Some("text/html")).1,
            "UTF-8",
            "{head:?}"
        );
    }
}

#[test]
fn a_meta_that_names_utf16_means_utf8() {
    // The scan reads the page as ASCII-compatible text, so it cannot be UTF-16; the standard
    // says to read it as UTF-8.
    let (text, name) = decoded(b"<meta charset=utf-16>caf\xC3\xA9", None);
    assert_eq!(
        (text.as_str(), name),
        ("<meta charset=utf-16>caf\u{e9}", "UTF-8")
    );
    let (_, name) = decoded(b"<meta charset=utf-16be>", None);
    assert_eq!(name, "UTF-8");
}

#[test]
fn decoding_never_fails() {
    let labels = [
        None,
        Some("text/html"),
        Some("text/html; charset=utf-8"),
        Some("text/html; charset=windows-1252"),
        Some("text/html; charset=shift_jis"),
        Some("text/html; charset=gbk"),
        Some("text/html; charset=utf-16le"),
        Some("text/html; charset=utf-16be"),
        Some("text/html; charset=iso-2022-jp"),
        Some("text/html; charset=x-user-defined"),
        Some("text/html; charset=\u{0}"),
    ];
    for label in labels {
        for byte in 0..=255_u8 {
            let _ = decode(&[byte], label);
            let _ = decode(&[byte, byte, byte, byte], label);
            let _ = decode(&[0xFF, 0xFE, byte], label);
            let _ = decode(&[0xEF, 0xBB, 0xBF, byte], label);
            let _ = decode(
                &[
                    b'<', b'm', b'e', b't', b'a', b' ', b'c', b'h', b'a', b'r', byte,
                ],
                label,
            );
        }
        let all: Vec<u8> = (0..=255).cycle().take(100_000).collect();
        let _ = decode(&all, label);
        // Markup that is cut or odd at every length up to the scan limit.
        let page = b"<!-- <meta <meta charset=\"x <meta http-equiv=content-type content=charset=";
        for length in 0..page.len() {
            let _ = decode(&page[..length], label);
        }
    }
}
