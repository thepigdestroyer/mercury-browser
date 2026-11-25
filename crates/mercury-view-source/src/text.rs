// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Bytes to text, and what kind of text.

use encoding_rs::{Encoding, REPLACEMENT, UTF_8, UTF_16BE, UTF_16LE};

use crate::source::SourceKind;

/// How many leading bytes `classify` looks at for a NUL byte or a leading `<`, and how many
/// `decode` searches for a `<meta>` declaration.
pub(crate) const SNIFF_BYTES: usize = 1024;

/// What a response holds, from its `Content-Type` and its first bytes (a NUL byte in the first
/// KiB, or an image, audio, video, font, PDF or octet-stream type, is [`SourceKind::Binary`]).
///
/// A known text type decides by itself. An unknown or missing type is sniffed for a leading `<`
/// (HTML) and nothing else, so it is [`SourceKind::Html`] or [`SourceKind::Plain`]. Text that
/// declares or starts with a UTF-16 byte order mark is full of NUL bytes and is not binary.
pub fn classify(content_type: Option<&str>, first_bytes: &[u8]) -> SourceKind {
    let essence = content_type.map(essence_of).unwrap_or_default();
    if is_binary_type(&essence) {
        return SourceKind::Binary;
    }
    let head = &first_bytes[..first_bytes.len().min(SNIFF_BYTES)];
    if head.contains(&0) && !is_utf16(content_type, head) {
        return SourceKind::Binary;
    }
    match essence.as_str() {
        "text/html" | "application/xhtml+xml" => SourceKind::Html,
        "text/css" => SourceKind::Css,
        "image/svg+xml" | "application/xml" | "text/xml" => SourceKind::Xml,
        _ if is_javascript_type(&essence) => SourceKind::JavaScript,
        _ if essence.ends_with("+xml") => SourceKind::Xml,
        _ if essence.starts_with("text/") || is_json_type(&essence) => SourceKind::Plain,
        // Unknown, or no type at all: only a leading `<` is believed.
        _ if starts_like_markup(head) => SourceKind::Html,
        _ => SourceKind::Plain,
    }
}

/// The text of `bytes` and the name of the encoding used: the header's charset, else a byte order
/// mark, else `<meta charset>` in the first KiB, else UTF-8. Never fails (invalid sequences
/// become U+FFFD).
///
/// A byte order mark wins over the header, as it does in a browser (one that agrees with the
/// header is only removed). An unknown charset name is ignored.
pub fn decode(bytes: &[u8], content_type: Option<&str>) -> (String, &'static str) {
    let from_header = declared_encoding(content_type);
    let (encoding, skip) = match (from_header, Encoding::for_bom(bytes)) {
        (_, Some((mark, length))) => (mark, length),
        (Some(header), None) => (header, 0),
        (None, None) => (meta_charset(bytes).unwrap_or(UTF_8), 0),
    };
    let body = bytes.get(skip..).unwrap_or_default();
    let (text, _had_errors) = encoding.decode_without_bom_handling(body);
    (text.into_owned(), encoding.name())
}

/// `text/html` from ` Text/HTML ; charset=utf-8`: the type without parameters, trimmed and in
/// lower case.
fn essence_of(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn is_binary_type(essence: &str) -> bool {
    if essence == "image/svg+xml" {
        return false;
    }
    const PREFIXES: [&str; 4] = ["image/", "audio/", "video/", "font/"];
    const TYPES: [&str; 14] = [
        "application/octet-stream",
        "application/pdf",
        "application/zip",
        "application/gzip",
        "application/x-gzip",
        "application/x-tar",
        "application/x-7z-compressed",
        "application/x-rar-compressed",
        "application/java-archive",
        "application/wasm",
        "application/x-shockwave-flash",
        "application/vnd.ms-fontobject",
        "application/font-woff",
        "application/font-sfnt",
    ];
    PREFIXES.iter().any(|prefix| essence.starts_with(prefix))
        || TYPES.contains(&essence)
        || essence.starts_with("application/x-font-")
}

/// The JavaScript MIME type essences of the HTML standard.
fn is_javascript_type(essence: &str) -> bool {
    matches!(
        essence,
        "application/ecmascript"
            | "application/javascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "text/ecmascript"
            | "text/javascript"
            | "text/javascript1.0"
            | "text/javascript1.1"
            | "text/javascript1.2"
            | "text/javascript1.3"
            | "text/javascript1.4"
            | "text/javascript1.5"
            | "text/jscript"
            | "text/livescript"
            | "text/x-ecmascript"
            | "text/x-javascript"
    )
}

fn is_json_type(essence: &str) -> bool {
    essence == "application/json" || essence.ends_with("+json")
}

/// A `<` as the first character that is not white space (after a UTF-8 byte order mark).
fn starts_like_markup(head: &[u8]) -> bool {
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    head.iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'<')
}

/// The response says it is UTF-16 (a `charset` parameter or a byte order mark).
fn is_utf16(content_type: Option<&str>, head: &[u8]) -> bool {
    let declared = declared_encoding(content_type);
    if matches!(declared, Some(encoding) if encoding == UTF_16LE || encoding == UTF_16BE) {
        return true;
    }
    matches!(
        Encoding::for_bom(head),
        Some((encoding, _)) if encoding == UTF_16LE || encoding == UTF_16BE
    )
}

/// The encoding the `charset` parameter of a `Content-Type` header names, if it names one that
/// is known. The `replacement` encodings (HZ, ISO-2022-KR and friends, which a browser shows as
/// one U+FFFD) count as unknown: the source is shown as UTF-8 text instead.
fn declared_encoding(content_type: Option<&str>) -> Option<&'static Encoding> {
    content_type
        .and_then(charset_parameter)
        .and_then(|label| known_encoding(&label))
}

/// The encoding a label names, unless the label is unknown or names a `replacement` encoding.
fn known_encoding(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.trim().as_bytes()).filter(|encoding| *encoding != REPLACEMENT)
}

/// The value of the `charset` parameter of a `Content-Type` header, without quotes.
fn charset_parameter(content_type: &str) -> Option<String> {
    content_type.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("charset") {
            return None;
        }
        let value = value.trim().trim_matches(['"', '\'']).trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

/// The encoding a `<meta charset=...>` or `<meta http-equiv="content-type" content="...;
/// charset=...">` in the first KiB names. A scan, not the whole HTML pre-scan algorithm: comments
/// are skipped, attribute values may be quoted or not, and a UTF-16 name means UTF-8 (the page
/// is being read as ASCII-compatible text, so it cannot be UTF-16), as the standard says.
fn meta_charset(bytes: &[u8]) -> Option<&'static Encoding> {
    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    let mut position = 0;
    while let Some(offset) = head[position..].iter().position(|byte| *byte == b'<') {
        position += offset;
        let rest = &head[position..];
        if rest.starts_with(b"<!--") {
            position += find_bytes(&rest[4..], b"-->").map_or(rest.len(), |end| 4 + end + 3);
            continue;
        }
        if rest.len() > 5
            && rest[1..5].eq_ignore_ascii_case(b"meta")
            && matches!(rest[5], b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/')
        {
            // A tag that is not closed within the first KiB is not read: its values may be cut.
            let tag_end = rest.iter().position(|byte| *byte == b'>')?;
            if let Some(encoding) = encoding_of_meta(&rest[5..tag_end]) {
                return Some(if encoding == UTF_16LE || encoding == UTF_16BE {
                    UTF_8
                } else {
                    encoding
                });
            }
            position += tag_end;
            continue;
        }
        position += 1;
    }
    None
}

/// The encoding named by the attributes of one `<meta>` tag.
fn encoding_of_meta(attributes: &[u8]) -> Option<&'static Encoding> {
    let mut charset = None;
    let mut content = None;
    let mut http_equiv_is_content_type = false;
    for (name, value) in attribute_pairs(attributes) {
        match name.as_str() {
            "charset" if charset.is_none() => charset = Some(value),
            "content" if content.is_none() => content = Some(value),
            "http-equiv" if value.eq_ignore_ascii_case("content-type") => {
                http_equiv_is_content_type = true;
            }
            _ => {}
        }
    }
    if let Some(label) = charset
        && let Some(encoding) = known_encoding(&label)
    {
        return Some(encoding);
    }
    if http_equiv_is_content_type
        && let Some(content) = content
        && let Some(label) = charset_in_content(&content)
    {
        return known_encoding(&label);
    }
    None
}

/// `charset=utf-8` inside a `content` attribute such as `text/html; charset=utf-8`.
fn charset_in_content(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let after = &content[lower.find("charset")? + "charset".len()..];
    let after = after.trim_start().strip_prefix('=')?.trim_start();
    let value: String = match after.chars().next()? {
        quote @ ('"' | '\'') => after[1..].chars().take_while(|c| *c != quote).collect(),
        _ => after
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != ';')
            .collect(),
    };
    (!value.is_empty()).then_some(value)
}

/// The `name=value` pairs of a tag's attributes, names in lower case, values unquoted. Bytes
/// that are not UTF-8 are replaced; encoding names and `http-equiv` values are ASCII.
fn attribute_pairs(attributes: &[u8]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut i = 0;
    let n = attributes.len();
    let is_space = |byte: u8| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/');
    while i < n {
        while i < n && is_space(attributes[i]) {
            i += 1;
        }
        let name_start = i;
        while i < n && !is_space(attributes[i]) && attributes[i] != b'=' {
            i += 1;
        }
        let name = String::from_utf8_lossy(&attributes[name_start..i]).to_ascii_lowercase();
        while i < n && is_space(attributes[i]) {
            i += 1;
        }
        if i >= n || attributes[i] != b'=' {
            if !name.is_empty() {
                pairs.push((name, String::new()));
            }
            continue;
        }
        i += 1;
        while i < n && is_space(attributes[i]) {
            i += 1;
        }
        let value = match attributes.get(i) {
            Some(&quote @ (b'"' | b'\'')) => {
                i += 1;
                let start = i;
                while i < n && attributes[i] != quote {
                    i += 1;
                }
                let value = String::from_utf8_lossy(&attributes[start..i]).into_owned();
                i += 1;
                value
            }
            _ => {
                let start = i;
                while i < n && !attributes[i].is_ascii_whitespace() {
                    i += 1;
                }
                String::from_utf8_lossy(&attributes[start..i]).into_owned()
            }
        };
        if !name.is_empty() {
            pairs.push((name, value));
        }
    }
    pairs
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charset_parameters_are_found_whatever_their_spelling() {
        for (header, expected) in [
            ("text/html; charset=utf-8", Some("utf-8")),
            ("text/html;CHARSET=\"Shift_JIS\"", Some("Shift_JIS")),
            ("text/html; format=flowed; charset = 'x'", Some("x")),
            ("text/html", None),
            ("text/html; charset=", None),
            ("text/html; chars=utf-8", None),
        ] {
            assert_eq!(charset_parameter(header).as_deref(), expected, "{header}");
        }
    }

    #[test]
    fn a_meta_scan_stops_at_the_first_kilobyte() {
        let mut page = b"<html><head>".to_vec();
        page.extend(std::iter::repeat_n(b' ', SNIFF_BYTES));
        page.extend_from_slice(b"<meta charset=windows-1251>");
        assert_eq!(meta_charset(&page), None);
    }

    #[test]
    fn a_meta_inside_a_comment_is_not_read() {
        let page = b"<!-- <meta charset=windows-1251> --><meta charset=shift_jis>";
        assert_eq!(meta_charset(page).map(Encoding::name), Some("Shift_JIS"));
        assert_eq!(
            meta_charset(b"<!-- <meta charset=windows-1251>").map(Encoding::name),
            None
        );
    }
}
