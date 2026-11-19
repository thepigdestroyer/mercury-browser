/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: refuses SVG images that are nested too deeply (decision D8).
//!
//! `usvg` converts an SVG document with functions that call themselves once per level of nesting, so an
//! image nested a hundred thousand elements deep (`<img src="...">` of a tiny file) can exhaust the stack
//! of the image decoder thread and take the whole browser down. Servo fixed this in servo/servo#49835 by
//! setting `usvg::Options::max_depth`, an option that exists from usvg 0.50. This tree has usvg 0.48.1,
//! and moving resvg also touches `servo-malloc-size-of`, which Mercury does not carry. This file does the
//! same job with one pass over the bytes, before `usvg` sees them.
//!
//! The scan never recurses, so its own cost does not depend on the nesting: it reads each byte once. It is
//! a lexer, not an XML parser. A document that is not XML at all, or that is cut off in the middle of a
//! construct, is reported as not too deep and left to `usvg`, which rejects it with its own error.
//!
//! Entities declared in a `<!DOCTYPE>` can expand to elements, which this scan does not follow. roxmltree
//! (the XML parser inside usvg) limits entity expansion to 10 levels, so that adds at most 10 to the
//! depth. Mercury's limit is 500, upstream's constant.
//!
//! This is a Mercury file. Drop it (with the call in `image_cache.rs` and the `mod mercury_svg;` line in
//! `lib.rs`) at the Servo bump that takes resvg 0.50, and set `max_depth: 500` there as upstream does.

use std::borrow::Cow;

use resvg::usvg;

/// The deepest nesting of elements Mercury accepts: the root is depth 1. Upstream's constant (servo#49835).
pub(crate) const MAX_SVG_DEPTH: u32 = 500;

/// The error `image_cache.rs` reports for a document that is too deeply nested.
pub(crate) const TOO_DEEP: &str = "SVG is nested too deeply";

/// The two magic bytes at the start of a gzip stream (an `.svgz` file).
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Unpacks `bytes` if it is gzip (SVGZ), then refuses it when it is nested deeper than [`MAX_SVG_DEPTH`].
/// On success the bytes to give `usvg::Tree::from_data` are returned: the unpacked document for SVGZ (so
/// usvg does not unpack it a second time), else `bytes` itself.
pub(crate) fn guard(bytes: &[u8]) -> Result<Cow<'_, [u8]>, &'static str> {
    let source = if bytes.starts_with(&GZIP_MAGIC) {
        match usvg::decompress_svgz(bytes) {
            // A gzip stream inside a gzip stream is not an SVG; usvg reports that from the original bytes.
            Ok(unpacked) if !unpacked.starts_with(&GZIP_MAGIC) => Cow::Owned(unpacked),
            // Malformed gzip: usvg reports it itself.
            _ => Cow::Borrowed(bytes),
        }
    } else {
        Cow::Borrowed(bytes)
    };
    if nesting_too_deep(&source) {
        Err(TOO_DEEP)
    } else {
        Ok(source)
    }
}

/// Whether the XML in `bytes` nests elements deeper than [`MAX_SVG_DEPTH`].
///
/// One pass, no recursion. Comments, CDATA sections, processing instructions and `<!DOCTYPE>` declarations
/// (with their internal subset) are skipped, and `<` or `>` inside a quoted attribute value is text. An
/// element that closes itself (`<a/>`) is one level deep at that point and does not raise the depth for
/// what follows. The depth of a leaf counts: 499 open elements and a self-closing one inside them is 500.
pub(crate) fn nesting_too_deep(bytes: &[u8]) -> bool {
    let limit = MAX_SVG_DEPTH as usize;
    let mut depth = 0usize;
    let mut at = 0usize;
    while let Some(offset) = find_byte(bytes, at, b'<') {
        let lt = at + offset;
        let Some(&next) = bytes.get(lt + 1) else {
            return false;
        };
        at = match next {
            // `<!--`, `<![CDATA[` and `<!DOCTYPE ... >`.
            b'!' => match end_of_bang(bytes, lt) {
                Some(end) => end,
                None => return false,
            },
            // A processing instruction, such as `<?xml version="1.0"?>`.
            b'?' => match find_seq(bytes, lt + 2, b"?>") {
                Some(end) => end + 2,
                None => return false,
            },
            // A close tag. Unbalanced ones are the parser's problem; the depth does not go below zero.
            b'/' => match find_byte(bytes, lt + 2, b'>') {
                Some(offset) => {
                    depth = depth.saturating_sub(1);
                    lt + 2 + offset + 1
                },
                None => return false,
            },
            // A start tag: the first character of an XML name.
            c if is_name_start(c) => match end_of_start_tag(bytes, lt + 1) {
                Some((gt, self_closing)) => {
                    if depth + 1 > limit {
                        return true;
                    }
                    if !self_closing {
                        depth += 1;
                    }
                    gt + 1
                },
                None => return false,
            },
            // A `<` that starts nothing. XML forbids it in text; the parser rejects the document.
            _ => lt + 1,
        };
    }
    false
}

/// The first XML name character: a letter, `_`, `:` or the first byte of a non-ASCII character.
fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b':' || byte >= 0x80
}

/// The offset of the first `needle` in `bytes[from..]`, relative to `from`.
fn find_byte(bytes: &[u8], from: usize, needle: u8) -> Option<usize> {
    bytes.get(from..)?.iter().position(|&byte| byte == needle)
}

/// The index in `bytes` of the first `needle` at or after `from`.
fn find_seq(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// The index just after the construct that starts with `<!` at `lt`: a comment, a CDATA section or a
/// declaration such as `<!DOCTYPE svg [ ... ]>`. `None` when it is cut off.
fn end_of_bang(bytes: &[u8], lt: usize) -> Option<usize> {
    let rest = &bytes[lt..];
    if rest.starts_with(b"<!--") {
        return find_seq(bytes, lt + 4, b"-->").map(|end| end + 3);
    }
    if rest.starts_with(b"<![CDATA[") {
        return find_seq(bytes, lt + 9, b"]]>").map(|end| end + 3);
    }
    // A declaration ends at the first `>` outside quotes and outside the `[ ... ]` internal subset, whose
    // own declarations end in `>` too. Comments and processing instructions inside the subset may hold
    // quotes and brackets, so they are skipped whole.
    let mut in_subset = false;
    let mut at = lt + 2;
    while at < bytes.len() {
        match bytes[at] {
            quote @ (b'"' | b'\'') => at += 1 + find_byte(bytes, at + 1, quote)?,
            b'<' if bytes[at..].starts_with(b"<!--") => at = find_seq(bytes, at + 4, b"-->")? + 2,
            b'<' if bytes[at..].starts_with(b"<?") => at = find_seq(bytes, at + 2, b"?>")? + 1,
            b'[' => in_subset = true,
            b']' => in_subset = false,
            b'>' if !in_subset => return Some(at + 1),
            _ => {},
        }
        at += 1;
    }
    None
}

/// For the start tag whose name begins at `from`: the index of its closing `>` and whether it closes
/// itself (`/>`). A `>` or `<` inside a quoted attribute value does not count. `None` when it is cut off.
fn end_of_start_tag(bytes: &[u8], from: usize) -> Option<(usize, bool)> {
    let mut quote: Option<u8> = None;
    let mut previous = 0u8;
    for (at, &byte) in bytes.get(from..)?.iter().enumerate() {
        match quote {
            Some(open) => {
                if byte == open {
                    quote = None;
                }
            },
            None => match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'>' => return Some((from + at, previous == b'/')),
                _ => {},
            },
        }
        previous = byte;
    }
    None
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    /// `levels` nested `<g>` elements inside `<svg>`'s own level: `levels` elements deep in all.
    fn nested(levels: usize) -> Vec<u8> {
        let mut svg = String::from(
            r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">"#,
        );
        for _ in 1..levels {
            svg.push_str("<g>");
        }
        for _ in 1..levels {
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");
        svg.into_bytes()
    }

    fn deep(text: &str) -> bool {
        nesting_too_deep(text.as_bytes())
    }

    #[test]
    fn the_limit_is_the_upstream_constant() {
        assert_eq!(MAX_SVG_DEPTH, 500);
    }

    #[test]
    fn the_boundary_is_500_levels() {
        assert!(!nesting_too_deep(&nested(499)));
        assert!(!nesting_too_deep(&nested(500)));
        assert!(nesting_too_deep(&nested(501)));
        assert!(nesting_too_deep(&nested(502)));
    }

    #[test]
    fn a_hundred_thousand_levels_are_refused_without_recursing() {
        // A recursive scan would overflow the 2 MiB stack of a test thread long before 100,000 levels.
        assert!(nesting_too_deep(&nested(100_000)));
        // Without a single close tag as well: the depth is reached on the way in.
        let open_only = "<g>".repeat(100_000);
        assert!(deep(&open_only));
    }

    #[test]
    fn a_wide_document_is_not_a_deep_one() {
        let flat = format!("<svg>{}</svg>", "<rect/>".repeat(100_000));
        assert!(!deep(&flat));
        let siblings = format!("<svg>{}</svg>", "<g></g>".repeat(100_000));
        assert!(!deep(&siblings));
    }

    #[test]
    fn a_self_closing_element_does_not_deepen_what_follows() {
        // 499 open elements and then 100 self-closing ones in a row: 500 deep at most.
        let doc = format!(
            "{}{}{}",
            "<g>".repeat(499),
            "<rect/>".repeat(100),
            "</g>".repeat(499)
        );
        assert!(!deep(&doc));
        // The leaf itself is a level: 500 open elements and a self-closing one inside them is 501.
        let doc = format!("{}<rect/>{}", "<g>".repeat(500), "</g>".repeat(500));
        assert!(deep(&doc));
        // `<g/>` repeated is never deeper than 1.
        assert!(!deep(&"<g/>".repeat(100_000)));
        // Space and attributes before the slash.
        assert!(!deep(&format!(
            "{}{}",
            "<g a=\"1\" />".repeat(1000),
            "<x/>"
        )));
    }

    #[test]
    fn close_tags_bring_the_depth_back() {
        let doc = format!(
            "{}{}{}",
            "<g>".repeat(400),
            "</g>".repeat(400),
            "<g>".repeat(400)
        );
        assert!(!deep(&doc));
        // More close tags than open ones do not wrap around to a large depth.
        let doc = format!("{}{}", "</g>".repeat(1000), "<g>".repeat(400));
        assert!(!deep(&doc));
    }

    #[test]
    fn comments_hide_markup() {
        let hidden = format!("<svg><!-- {} --></svg>", "<g>".repeat(10_000));
        assert!(!deep(&hidden));
        // `-->` ends the comment, and a `>` or `--` inside it does not.
        let doc = format!("<svg><!-- a > b -- c -->{}</svg>", "<g>".repeat(600));
        assert!(deep(&doc));
        // `<!-->` does not open and close a comment at once.
        let doc = format!("<svg><!--> {} --></svg>", "<g>".repeat(600));
        assert!(!deep(&doc));
    }

    #[test]
    fn cdata_hides_markup() {
        let hidden = format!(
            "<svg><style><![CDATA[ {} ]]></style></svg>",
            "<g>".repeat(10_000)
        );
        assert!(!deep(&hidden));
        // A `>` and a `]]` inside the section do not end it; `]]>` does.
        let doc = format!("<svg><![CDATA[ a ]] > b ]]>{}</svg>", "<g>".repeat(600));
        assert!(deep(&doc));
    }

    #[test]
    fn processing_instructions_hide_markup() {
        let hidden = format!(
            r#"<?xml version="1.0"?><?x {} ?><svg/>"#,
            "<g>".repeat(10_000)
        );
        assert!(!deep(&hidden));
        let doc = format!("<?xml-stylesheet href='a'?>{}", "<g>".repeat(600));
        assert!(deep(&doc));
    }

    #[test]
    fn a_doctype_with_an_internal_subset_is_skipped() {
        let doc = format!(
            r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [
                <!ENTITY a "<g>&b;</g>">
                <!ENTITY b 'x > y ] z'>
                <!-- <g><g> '"' ] > -->
                <?pi <g> ?>
            ]><svg>{}</svg>"#,
            "<g>".repeat(600)
        );
        assert!(deep(&doc), "the document after the subset is 601 deep");
        let doc = doc.replace(&"<g>".repeat(600), "<g>");
        assert!(!deep(&doc), "the markup inside the subset is not elements");
        // A DOCTYPE without a subset.
        assert!(!deep(r#"<!DOCTYPE svg><svg><g/></svg>"#));
    }

    #[test]
    fn quoted_attribute_values_may_hold_angle_brackets() {
        let doc = format!(
            r#"<svg a="<g><g><g>" b='>' c="/>" d='"<g>"'>{}</svg>"#,
            "<g>".repeat(10)
        );
        assert!(!deep(&doc));
        // A `/>` inside a quote does not make the tag self-closing: the element stays open.
        let doc = format!(r#"{}"#, r#"<g title="a/>">"#.repeat(501));
        assert!(deep(&doc));
        // And a `>` inside a quote does not end the tag early, so what follows is not a start tag.
        let doc = r#"<g title="></g><g><g><g>">"#.repeat(200);
        assert!(!deep(&doc));
        let doc = format!("{}{}", r#"<g a='" > <g>' >"#.repeat(499), "<rect/>");
        assert!(!deep(&doc));
    }

    #[test]
    fn text_between_the_elements_is_ignored() {
        let doc = format!(
            "{}a > b ]]> c{}",
            "<g>\n  ".repeat(300),
            "</g>\n".repeat(300)
        );
        assert!(!deep(&doc));
    }

    #[test]
    fn what_is_not_xml_is_left_to_usvg() {
        assert!(!deep(""));
        assert!(!deep("plain text, no markup at all"));
        assert!(!deep("<"));
        assert!(!deep("< <3 <1 <. <- <= < g>"));
        // Cut off in the middle of a construct: the document is not well formed, usvg says so.
        assert!(!deep("<svg><!-- never ends <g><g><g>"));
        assert!(!deep("<svg><![CDATA[ never ends"));
        assert!(!deep("<?xml never ends"));
        assert!(!deep("<!DOCTYPE svg [ <!ENTITY a \"never ends>"));
        assert!(!deep("<svg a=\"never ends>"));
        assert!(!deep("<svg></"));
        // A binary file that happens to hold `<`.
        assert!(!nesting_too_deep(&[
            0x89, b'P', b'N', b'G', b'<', 0x00, 0xff, b'<', 0x1a, b'>'
        ]));
    }

    #[test]
    fn names_with_namespaces_and_non_ascii_letters_count() {
        assert!(deep(&"<svg:g>".repeat(501)));
        assert!(deep(&"<_g>".repeat(501)));
        assert!(deep(&"<\u{e9}l>".repeat(501)));
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn the_guard_passes_an_ordinary_document_unchanged() {
        let svg = nested(10);
        match guard(&svg) {
            Ok(Cow::Borrowed(same)) => assert_eq!(same, svg.as_slice()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_guard_refuses_a_deep_document() {
        assert_eq!(guard(&nested(501)).unwrap_err(), "SVG is nested too deeply");
        assert!(guard(&nested(500)).is_ok());
    }

    #[test]
    fn an_svgz_file_is_unpacked_before_it_is_looked_at() {
        // Compressed, 100,000 levels are a few hundred bytes of gzip: this is the way round the check.
        let packed = gzip(&nested(100_000));
        assert!(packed.len() < 4096, "{} bytes", packed.len());
        assert_eq!(guard(&packed).unwrap_err(), TOO_DEEP);
        // The gzip bytes themselves hold no `<` worth reading.
        assert!(!nesting_too_deep(&packed));

        // A shallow one is returned unpacked, so usvg does not unpack it again.
        let svg = nested(20);
        let packed = gzip(&svg);
        match guard(&packed) {
            Ok(Cow::Owned(unpacked)) => assert_eq!(unpacked, svg),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_broken_or_doubled_gzip_stream_is_left_to_usvg() {
        let mut packed = gzip(&nested(20));
        packed.truncate(packed.len() / 2);
        // Not refused here: the original bytes go to usvg, which reports the gzip error.
        assert!(matches!(guard(&packed), Ok(Cow::Borrowed(same)) if same == packed.as_slice()));
        let doubled = gzip(&gzip(&nested(20)));
        assert!(matches!(guard(&doubled), Ok(Cow::Borrowed(same)) if same == doubled.as_slice()));
    }

    #[test]
    fn the_test_documents_are_real_svg_that_usvg_reads() {
        // The guard's verdicts above are about documents usvg accepts. Parse on a thread with a large stack
        // so this test does not itself depend on how much stack usvg takes per level.
        let parsed = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let options = usvg::Options::default();
                usvg::Tree::from_data(&nested(MAX_SVG_DEPTH as usize), &options).is_ok()
                    && usvg::Tree::from_data(&gzip(&nested(20)), &options).is_ok()
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(parsed);
    }
}
