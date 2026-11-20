// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Bytes to text, and what kind of text. Item 2 (source-fetch) fills in the bodies.

use crate::source::SourceKind;

/// What a response holds, from its `Content-Type` and its first bytes (a NUL byte in the first
/// KiB, or an image, audio, video, font, PDF or octet-stream type, is [`SourceKind::Binary`]).
pub fn classify(_content_type: Option<&str>, _first_bytes: &[u8]) -> SourceKind {
    todo!("item 2: source-fetch")
}

/// The text of `bytes` and the name of the encoding used: the header's charset, else a byte order
/// mark, else `<meta charset>` in the first KiB, else UTF-8. Never fails (invalid sequences
/// become U+FFFD).
pub fn decode(_bytes: &[u8], _content_type: Option<&str>) -> (String, &'static str) {
    todo!("item 2: source-fetch")
}
